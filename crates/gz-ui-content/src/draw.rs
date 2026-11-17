//! The browser drawn into a host: the toolbar, the Sources pane beside the tiles, the status
//! strip under both. Everything it shows comes from the catalogs, the view queries
//! and the pictures; everything it does is an action of [`crate::actions`].

use std::sync::Arc;

use egui::{
    Align2, CornerRadius, Id, Key, Modifiers, Popup, Pos2, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder,
    Vec2, pos2,
};
use gz_app::DocCx;
use gz_editor::Editor;
use gz_fmt_tex::TileAlpha;
use gz_ui::menu::{MenuItem, menu_item, menu_popup, take_popup_request};
use gz_ui::state::{badge, banner, busy_line, empty_state};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Font, Icon, Kind, NavRowSpec, Palette, TipExt, Toolbar, ToolbarStyle, Tx, icons, metrics, nav_row,
    palette, tr, units,
};
use rustc_hash::FxHashSet;

use gz_ui::drop::{Payload, Zone};
use gz_ui::section::Section;

use crate::actions::{self, ADD_ITEMS, BuildToggle};
use crate::caption;
use crate::catalog::{Catalog, Node, Origin, Picture, View, ViewEntry};
use crate::local::{local_breadcrumbs, local_marquee, local_menu_heading, local_rename_box, local_tree_row};
use crate::model::{
    GRID_PAD_Y, HOME, Host, KINDS, LIST_BELOW, LIST_COLUMN_EM, LIST_PICTURE, LIST_ROW, Location, Source,
    TILE_GAP, TILE_PAD, leaf_of, search_active, thousands,
};
use crate::pictures::{Bytes, PicKey, Shown};
use crate::select::Entry;
use crate::service::{CatState, ContentBrowser, Dragging, Inner, cat_key, tree_key};

/// What the frame knows of the editor around the browser.
struct Frame<'a> {
    editor: &'a Arc<Editor>,
    browser: &'a Arc<ContentBrowser>,
    ctx: egui::Context,
    host: Host,
    now: u64,
    has_project: bool,
    cat: CatState,
    p: Arc<Palette>,
}

impl Frame<'_> {
    fn toast<'t>(&self, text_: impl Into<Tx<'t>>) {
        gz_ui::state::toast(&self.ctx, gz_ui::Tone::Warning, text_);
    }
}

/// "Library / Textures / ui" in the window's language: where a favorite points.
fn location_label(ctx: &egui::Context, loc: &Location) -> String {
    let mut label = Tx::from(loc.source.label()).text(ctx);
    if let Some(k) = loc.lib_kind() {
        label.push_str(" / ");
        label.push_str(&Tx::from(k.caption).text(ctx));
    }
    for part in loc.folder.split('\\').filter(|p| !p.is_empty()) {
        label.push_str(" / ");
        label.push_str(part);
    }
    label
}

/// Draws the Content Browser into `ui`: the Level tab's panel and the Content Drawer both call
/// it, each with its own [`Host`].
pub fn content_browser(ui: &mut Ui, cx: &mut DocCx<'_>, host: Host) {
    let editor = cx.editor.clone();
    let browser = ContentBrowser::of(&editor);
    let ctx = ui.ctx().clone();
    let now = ctx.cumulative_frame_nr();
    let mut guard = browser.inner.lock();
    let st = &mut *guard;
    st.load_view(&ctx);
    // the panel while the drawer shows the browser only says so: one host a frame
    if host == Host::Panel && st.host(Host::Drawer).frame.is_some_and(|d| d + 1 >= now) {
        st.host_mut(host).frame = Some(now);
        empty_state(ui, Icon::Drawer, "In the Content Drawer", "Close the drawer to browse here.", None);
        return;
    }
    st.host_mut(host).frame = Some(now);
    let (project_root, game_root) = match gz_project::Project::of(&editor) {
        Ok((_, p)) => (p.root().to_owned(), p.game_root()),
        Err(_) => (String::new(), String::new()),
    };
    let has_project = !project_root.is_empty();
    st.sync_scope(&project_root, &game_root, &browser.cats);
    take_inbox(&ctx, &editor, st, &browser);
    st.pics.begin_frame(&ctx, now);
    // Libraries > Reload Textures: the tiles' pictures are read again
    let reload = gz_app::gpu::TextureReload::generation(&editor);
    let seen_id = egui::Id::unique("gz-ui-content.texture_reload");
    if ctx.data_mut(|d| d.get_temp::<u64>(seen_id)).is_some_and(|seen| seen != reload) {
        st.pics.forget_all();
    }
    ctx.data_mut(|d| d.insert_temp(seen_id, reload));
    if st.view_dirty {
        st.store_view(&ctx);
    }

    let key = cat_key(&st.loc);
    let cat = browser.cats.ensure(&editor, key);
    if let Some(c) = &cat.cat {
        keep_valid(st, c, cat.busy);
    }
    // a listing made again: the pictures of its files may be old
    if st.listed != Some((key, cat.rev)) {
        if st.listed.is_some_and(|(k, _)| k == key) {
            st.pics.forget_all();
        }
        st.listed = Some((key, cat.rev));
    }
    let f = Frame {
        editor: &editor,
        browser: &browser,
        ctx: ctx.clone(),
        host,
        now,
        has_project,
        cat,
        p: palette(&ctx),
    };
    let browser_rect = ui.max_rect();

    toolbar(ui, st, &f);
    body(ui, cx, st, &f);
    status_strip(ui, st, &f);
    shortcuts(&ctx, cx, st, &f, browser_rect);
    drag_preview(&ctx, st, &f);

    // the scroll to a selection made from outside has had both panes' frame, unless its folder
    // was still being listed
    st.scroll_to_selection = std::mem::take(&mut st.reveal_waits);
    let gpu = Some((&cx.gpu.device, &cx.gpu.queue));
    let raised = f.p.raised;
    st.pics.dispatch(
        &editor,
        &browser.thumbs,
        gpu,
        gz_render::Color::rgb(raised.r(), raised.g(), raised.b()),
    );
}

/// What jobs left for the frame: a rename box to open, a rename that landed, a deletion.
fn take_inbox(ctx: &egui::Context, editor: &Arc<Editor>, st: &mut Inner, browser: &ContentBrowser) {
    let mut inbox = browser.inbox.lock();
    if let Some(n) = inbox.imported.take()
        && n > 0
    {
        gz_ui::state::toast(ctx, gz_ui::Tone::Success, tr!("Made {n} {n:asset|assets}.", n = n));
    }
    if let Some(clash) = inbox.import_clash.take() {
        actions::ask_import_clash(ctx, editor, clash);
    }
    if let Some((was, now)) = inbox.renamed.take() {
        st.follow_rename(&was, &now);
    }
    if std::mem::take(&mut inbox.clear_selection) {
        st.sel.clear();
    }
    if let Some(e) = inbox.rename.take() {
        st.sel.only(e.clone());
        actions::begin_rename(st, &e);
        st.scroll_to_selection = true;
    }
}

/// What the listing no longer holds goes from the selection and the rename box; a folder shown
/// that went is left for the root.
fn keep_valid(st: &mut Inner, cat: &Catalog, busy: bool) {
    if busy {
        return;
    }
    if !st.loc.folder.is_empty() && cat.folder(&st.loc.folder).is_none() {
        st.loc.folder.clear();
        st.sel.clear();
        st.pics.cancel();
    }
    if !st.sel.is_empty() {
        st.sel.prune(|e| cat.contains(e));
    }
    if st.rename.as_ref().is_some_and(|r| !cat.contains(&r.entry)) {
        st.rename = None;
    }
}

/// Goes somewhere from a click; a refusal is a toast.
fn go(st: &mut Inner, f: &Frame<'_>, target: Location) {
    let before = st.loc.clone();
    if let Err(e) = st.apply_location(target, true, &f.browser.cats, f.has_project) {
        // the browser's own English
        f.toast(Tx::ui(e));
    }
    if !st.loc.same(&before) || st.loc.source != before.source {
        st.pics.cancel();
    }
}

fn nav_result(st: &mut Inner, f: &Frame<'_>, r: Result<(), String>, before: &Location) {
    if let Err(e) = r {
        // the browser's own English
        f.toast(Tx::ui(e));
    }
    if !st.loc.same(before) {
        st.pics.cancel();
    }
}

// ---- the toolbar ------------------------------------------------------------------------------

/// [+ Add] | Back Forward Up | breadcrumb | search | View | star | Favorites.
fn toolbar(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>) {
    let row_w = ui.available_width();
    let font = Font::Body.id();
    let (crumb_root, crumb_folder) = crumb_path(&f.ctx, &st.loc);
    let crumbs_full: f32 = text::width(&f.ctx, &font, Tx::raw(crumb_root.as_str()))
        + 12.0
        + crumb_folder
            .split('\\')
            .filter(|s| !s.is_empty())
            .map(|s| 16.0 + text::width(&f.ctx, &font, Tx::raw(s)) + 12.0)
            .sum::<f32>();
    let menu_w = |label: &'static str| 56.0 + text::width(&f.ctx, &font, label);
    let others = menu_w("Add")
        + 13.0
        + 28.0 * 3.0
        + 4.0
        + 13.0
        + 14.0
        + units::em(10.0)
        + 12.0
        + menu_w("View")
        + 12.0
        + 30.0
        + menu_w("Favorites")
        + 8.0;
    let crumbs_w = (row_w - others).min(crumbs_full).max(units::em(6.0));

    let add_off = st.add_blocked(f.has_project);
    let mut add_pick: Option<usize> = None;
    let mut crumb_go: Option<String> = None;
    let mut search = st.search.clone();
    let favorite = st.find_favorite(&st.loc);
    let can_favorite =
        !st.loc.home() && (favorite.is_some() || st.loc.source != Source::Project || f.has_project);
    let (back, forward, up, star);
    let (back_on, forward_on, up_on) = (
        !st.back.is_empty(),
        !st.forward.is_empty(),
        !st.loc.folder.is_empty() || (st.loc.source == Source::Library && !st.loc.home()),
    );
    let root_label = crumb_root.clone();
    let folder = crumb_folder.clone();
    {
        let mut tb = Toolbar::begin(ui, "cb_toolbar", ToolbarStyle::Panel);
        tb.group(3);
        tb.menu(
            "cb_add",
            Icon::Plus,
            "Add",
            add_off.unwrap_or("A folder or an asset in the folder shown"),
            Some("cb_add"),
            add_off.is_none(),
            |ui| add_menu(ui, add_off, &mut add_pick),
        );
        tb.separator();
        tb.group(4);
        back = tb.action("cb_back", Icon::Back, "", "Back (Alt+Left)", back_on, false, Kind::Ghost);
        forward =
            tb.action("cb_forward", Icon::Forward, "", "Forward (Alt+Right)", forward_on, false, Kind::Ghost);
        up = tb.action("cb_up", Icon::Up, "", "Parent Folder (Alt+Up)", up_on, false, Kind::Ghost);
        tb.separator();
        tb.group(5);
        tb.widget("cb_breadcrumb", crumbs_w, |ui| crumb_go = local_breadcrumbs(ui, &root_label, &folder));
        tb.spring();
        tb.group(2);
        tb.search(&mut search, "Search", 10.0);
        tb.group(1);
        tb.menu(
            "cb_view",
            Icon::ViewOptions,
            "View",
            "Tiles or list, the tile size, the source and the Sources pane",
            Some("cb_view"),
            true,
            |ui| view_menu(ui, st, f),
        );
        tb.group(0);
        star = tb.action(
            "cb_star",
            if favorite.is_some() { Icon::StarFilled } else { Icon::Star },
            "",
            if favorite.is_some() { "Remove from Favorites" } else { "Add to Favorites" },
            can_favorite,
            favorite.is_some(),
            Kind::Ghost,
        );
        tb.menu(
            "cb_favorites",
            Icon::Bookmark,
            "Favorites",
            "Favorite folders",
            Some("cb_favorites"),
            true,
            |ui| favorites_menu(ui, st, f),
        );
        tb.end();
    }
    if search != st.search {
        st.search = search;
        st.pics.cancel();
    }
    let before = st.loc.clone();
    if back {
        let r = st.go_back(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if forward {
        let r = st.go_forward(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if up {
        let r = st.go_up(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if let Some(path) = crumb_go {
        let target = crumb_target(&st.loc, &path);
        go(st, f, target);
    }
    if star {
        match favorite {
            Some(i) => {
                st.favorites.remove(i);
            }
            None => {
                let here = st.loc.clone();
                if let Err(e) = st.add_favorite(here, &f.browser.cats, f.has_project) {
                    f.toast(tr!("The folder cannot be a favorite: {error}", error = Tx::ui(e)));
                }
            }
        }
    }
    if let Some(i) = add_pick {
        run_add(st, f, i);
    }
}

/// The breadcrumb of a location as the window shows it: a Library kind's folders hang under
/// "Library" and the kind (a click on the kind's crumb goes to the kind's root whatever it reads).
fn crumb_path(ctx: &egui::Context, loc: &Location) -> (String, String) {
    let word = |en: &'static str| Tx::from(en).text(ctx);
    match loc.lib_kind() {
        Some(k) if loc.folder.is_empty() => (word("Library"), word(k.caption)),
        Some(k) => (word("Library"), format!("{}\\{}", word(k.caption), loc.folder)),
        None => (word(loc.root_label()), loc.folder.clone()),
    }
}

/// Where a click on a breadcrumb goes.
fn crumb_target(loc: &Location, path: &str) -> Location {
    if loc.source != Source::Library {
        return Location { folder: path.to_owned(), ..loc.clone() };
    }
    match path.split_once('\\') {
        _ if path.is_empty() => Location::new(Source::Library, HOME, ""),
        Some((_, rest)) => Location::new(Source::Library, loc.category, rest),
        None => Location::new(Source::Library, loc.category, ""),
    }
}

fn add_menu(ui: &mut Ui, blocked: Option<&'static str>, pick: &mut Option<usize>) {
    for (i, (label, _tip, kind)) in ADD_ITEMS.iter().enumerate() {
        if i == 2 {
            ui.separator();
        }
        let icon = match *kind {
            None => Icon::Folder,
            Some(actions::IMPORT) => Icon::Open,
            Some("quest") => Icon::Quest,
            Some("behaviour") => Icon::Behaviour,
            Some("character") => Icon::Character,
            Some("item") => Icon::Item,
            Some("trader") => Icon::Trader,
            Some("loot") => Icon::Loot,
            Some("recipes") => Icon::Recipe,
            _ => Icon::PdaInterface,
        };
        if menu_item(
            ui,
            *label,
            MenuItem {
                icon,
                enabled: blocked.is_none(),
                tip: blocked.unwrap_or("").into(),
                ..MenuItem::default()
            },
        ) {
            *pick = Some(i);
        }
    }
}

fn run_add(st: &Inner, f: &Frame<'_>, i: usize) {
    if st.add_blocked(f.has_project).is_some() {
        return;
    }
    match ADD_ITEMS.get(i).and_then(|(_, _, kind)| *kind) {
        Some(actions::IMPORT) => actions::import_dialog(f.editor, f.browser, &st.loc.folder),
        Some(kind) => actions::create(f.editor, f.browser, st, kind),
        None => actions::new_folder(f.editor, f.browser, st),
    }
}

fn view_menu(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>) {
    let host = f.host;
    local_menu_heading(ui, "View As");
    let list = st.host(host).list;
    if menu_item(ui, "Tiles", MenuItem { selected: !list, ..MenuItem::default() }) {
        st.host_mut(host).list = false;
        st.view_dirty = true;
    }
    if menu_item(ui, "List", MenuItem { selected: list, ..MenuItem::default() }) {
        st.host_mut(host).list = true;
        st.view_dirty = true;
    }
    local_menu_heading(ui, "Tile Size");
    let fit = st.host(host).tile <= 0.0;
    if menu_item(ui, "Fit to Height", MenuItem { selected: fit, ..MenuItem::default() }) {
        // off keeps the size shown, now as the one picked
        let v = st.host_mut(host);
        v.tile = if fit { v.shown_tile(host) } else { 0.0 };
        v.list = false;
        st.view_dirty = true;
    }
    let mut size = st.host(host).shown_tile(host).max(LIST_BELOW);
    // the row is as wide as its field: the popup keeps the width of its longest label
    // placed by hand: a menu's justified layout would stretch the field across the screen
    let row = Rect::from_min_size(
        ui.cursor().min + Vec2::new(8.0, 0.0),
        Vec2::new(units::em(10.0), metrics::FRAME),
    );
    ui.allocate_rect(
        Rect::from_min_size(ui.cursor().min, Vec2::new(units::em(10.0) + 8.0, metrics::FRAME)),
        Sense::hover(),
    );
    ui.scope_builder(UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        let resp = gz_ui::grid::number(ui, &mut size, 8.0, "pt").tip(
            "The side of a tile; the panel and the Content Drawer keep one each. Ctrl+wheel over the tiles changes it too; under 64 pt the folder is a list.",
        );
        if resp.changed() {
            let v = st.host_mut(host);
            v.tile = size.clamp(LIST_BELOW, gz_app::contract::TILE_MAX);
            v.list = false;
            st.view_dirty = true;
        }
    });
    local_menu_heading(ui, "Source");
    for s in Source::ALL {
        if menu_item(ui, s.label(), MenuItem { selected: st.loc.source == s, ..MenuItem::default() }) {
            let before = st.loc.clone();
            st.switch_source(s, &f.browser.cats, f.has_project);
            nav_result(st, f, Ok(()), &before);
        }
    }
    ui.separator();
    if menu_item(ui, "Sources Pane", MenuItem { selected: st.sources_pane, ..MenuItem::default() }) {
        st.sources_pane = !st.sources_pane;
        st.view_dirty = true;
    }
    if menu_item(ui, "Refresh", MenuItem { keys: "F5", ..MenuItem::default() }) {
        refresh(st, f);
    }
}

fn refresh(st: &mut Inner, f: &Frame<'_>) {
    f.browser.cats.refresh(f.editor, cat_key(&st.loc));
    st.pics.forget_all();
}

fn favorites_menu(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>) {
    if st.favorites.is_empty() {
        ui.set_max_width(units::em(18.0));
        gz_ui::text::status(
            ui,
            gz_ui::Tone::Muted,
            "No favorites yet. The star beside this menu keeps the folder shown.",
        );
        return;
    }
    ui.set_min_width(units::em(22.0));
    let mut open: Option<usize> = None;
    let mut remove: Option<usize> = None;
    for (i, fav) in st.favorites.iter().enumerate() {
        let label = location_label(&f.ctx, fav);
        let spec = NavRowSpec {
            icon: Icon::StarFilled,
            tip: Tx::raw(label.as_str()),
            action: Icon::Close,
            action_tip: "Remove from Favorites".into(),
            ..NavRowSpec::default()
        };
        let r = nav_row(ui, &format!("cb_fav_menu_{i}"), Tx::raw(label.as_str()), fav.same(&st.loc), &spec);
        if r.action {
            remove = Some(i);
        } else if r.clicked {
            open = Some(i);
        }
    }
    if let Some(i) = remove {
        st.favorites.remove(i);
    } else if let Some(i) = open
        && let Some(fav) = st.favorites.get(i).cloned()
    {
        let before = st.loc.clone();
        if let Err(e) = st.apply_location(fav, true, &f.browser.cats, f.has_project) {
            f.toast(tr!("The favorite cannot be opened: {error}", error = Tx::ui(e)));
        }
        nav_result(st, f, Ok(()), &before);
        ui.close();
    }
}

// ---- the body: the Sources pane and the tiles ---------------------------------------------------

fn body(ui: &mut Ui, cx: &mut DocCx<'_>, st: &mut Inner, f: &Frame<'_>) {
    let status_h = 24.0;
    let avail = ui.available_size();
    let width = avail.x;
    let body_h = (avail.y - status_h).floor().max(48.0);
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(width, body_h));
    ui.allocate_rect(rect, Sense::hover());
    st.sources_fit = width >= 480.0;
    st.sources_shown = st.sources_pane && st.sources_fit;
    let mut row = ui.new_child(UiBuilder::new().id_salt(("cb_body", f.host.index())).max_rect(rect));
    let grid_rect = if st.sources_shown {
        let split = gz_ui::splitter::splitter(&mut row, "content_browser.sources", true, 160.0, 200.0, 220.0);
        let gap = row.spacing().item_spacing.x;
        let pane = Rect::from_min_size(rect.min, Vec2::new(split.size, rect.height()));
        let mut pane_ui = row.new_child(
            UiBuilder::new()
                .id_salt(("cb_sources", f.host.index()))
                .max_rect(pane.shrink2(Vec2::new(0.0, 4.0))),
        );
        sources(&mut pane_ui, st, f);
        Rect::from_min_max(pos2(pane.max.x + gap, rect.min.y), rect.max)
    } else {
        rect
    };
    // what the tiles of a host with no size picked fit: measured here, outside the scrolling grid
    st.host_mut(f.host).grid_h = (grid_rect.height() - GRID_PAD_Y * 2.0).max(0.0);
    row.painter().rect_filled(grid_rect, CornerRadius::ZERO, f.p.inset);
    let inner = Rect::from_min_max(
        grid_rect.min + Vec2::new(8.0, GRID_PAD_Y),
        grid_rect.max - Vec2::new(8.0, GRID_PAD_Y),
    );
    let mut grid = row.new_child(UiBuilder::new().id_salt(("cb_grid", f.host.index())).max_rect(inner));
    grid_body(&mut grid, cx, st, f, grid_rect);
    entry_menu(st, f, row.layer_id());
}

fn sources(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>) {
    let mut idx = st.loc.source.index();
    if gz_ui::segmented(ui, &mut idx, &["Project", "Library"], None) {
        let before = st.loc.clone();
        st.switch_source(Source::ALL.get(idx).copied().unwrap_or_default(), &f.browser.cats, f.has_project);
        nav_result(st, f, Ok(()), &before);
    }
    ui.add_space(2.0);
    // a pane that cannot hold six tree rows beside the two headers keeps that room for the tree
    let sections = ui.available_height() >= 28.0 * 2.0 + 22.0 * 6.0;
    let veils = f.browser.veils(f.editor, &st.loc, &f.cat);
    ScrollArea::vertical()
        .id_salt(("cb_sources_scroll", f.host.index(), st.loc.source.index()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if sections {
                let count = st.favorites.len().to_string();
                let key = format!("content_browser.favorites_{}", f.host.name());
                let head = Section::new(&key, "Favorites")
                    .default_open(!st.favorites.is_empty())
                    .count(Tx::raw(if st.favorites.is_empty() { "" } else { count.as_str() }));
                if head.show(ui).open {
                    if st.favorites.is_empty() {
                        gz_ui::text::status(
                            ui,
                            gz_ui::Tone::Muted,
                            "The star on the toolbar keeps the folder shown here.",
                        );
                    }
                    favorite_rows(ui, st, f);
                }
                let key = format!("content_browser.folders_{}", f.host.name());
                if Section::new(&key, "Folders").show(ui).open {
                    tree(ui, st, f, veils.as_deref());
                }
            } else {
                if !st.favorites.is_empty() {
                    favorite_rows(ui, st, f);
                    ui.add_space(3.0);
                    ui.separator();
                    ui.add_space(3.0);
                }
                tree(ui, st, f, veils.as_deref());
            }
        });
}

fn favorite_rows(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>) {
    let mut open: Option<usize> = None;
    let mut remove: Option<usize> = None;
    for (i, fav) in st.favorites.iter().enumerate() {
        let leaf = leaf_of(&fav.folder);
        // a root is the source's or the kind's name, a folder its own
        let name = if leaf.is_empty() { Tx::from(fav.root_label()) } else { Tx::raw(leaf) };
        let tip = location_label(&f.ctx, fav);
        let spec = NavRowSpec {
            icon: Icon::StarFilled,
            tip: Tx::raw(tip.as_str()),
            action: Icon::Close,
            action_tip: "Remove from Favorites".into(),
            ..NavRowSpec::default()
        };
        let r = nav_row(ui, &format!("cb_fav_{}_{i}", f.host.index()), name, fav.same(&st.loc), &spec);
        if r.action {
            remove = Some(i);
        } else if r.clicked {
            open = Some(i);
        }
    }
    if let Some(i) = remove {
        st.favorites.remove(i);
    } else if let Some(fav) = open.and_then(|i| st.favorites.get(i).cloned()) {
        let before = st.loc.clone();
        if let Err(e) = st.apply_location(fav, true, &f.browser.cats, f.has_project) {
            f.toast(tr!("The favorite cannot be opened: {error}", error = Tx::ui(e)));
        }
        nav_result(st, f, Ok(()), &before);
    }
}

/// What a tree row asked for.
enum TreeAct {
    Open(Location),
    Menu(Entry, Pos2),
    Drop(Payload, String),
}

fn tree(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>, veils: Option<&FxHashSet<String>>) {
    let mut act: Option<TreeAct> = None;
    if st.loc.source == Source::Library {
        library_tree(ui, st, f, &mut act);
    } else {
        let Some(cat) = f.cat.cat.clone() else {
            if f.cat.busy {
                busy_line(ui, "Reading the folders.");
            }
            return;
        };
        // the root row names the project: the segment above already says "Project"
        let name =
            f.editor.project_root().and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()));
        let label = name.unwrap_or_else(|| gz_ui::i18n::t(&f.ctx, "Project"));
        tree_node(ui, st, f, &cat, cat.root(), 0, veils, &label, &mut act);
    }
    match act {
        Some(TreeAct::Open(target)) => {
            let path = target.folder.clone();
            let library = target.source == Source::Library;
            go(st, f, target);
            // a folder is an entry like any other: picked in the tree, it is selected
            if path.is_empty() || library {
                st.sel.clear();
            } else {
                st.sel.only(Entry::new(&path, true));
            }
        }
        Some(TreeAct::Menu(e, at)) => {
            if !st.sel.contains(&e) {
                st.sel.only(e.clone());
            }
            st.menu = Some((Some(e), at));
            Popup::open_id(&f.ctx, menu_id(f.host));
        }
        Some(TreeAct::Drop(payload, dst)) => take_drop(st, f, payload, &dst),
        None => {}
    }
}

/// The Library's tree: its kinds, the open one with its folders.
fn library_tree(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>, act: &mut Option<TreeAct>) {
    for (i, k) in KINDS.iter().enumerate() {
        let key = format!("library:{}", k.word);
        let shown = st.loc.category == i;
        let open = shown || st.tree_open.contains(&key);
        let id = Id::unique(("cb_tree_kind", f.host.index(), i));
        let row = local_tree_row(
            ui,
            id,
            0,
            Tx::from(k.caption),
            open,
            true,
            shown && st.loc.folder.is_empty(),
            false,
        );
        if row.toggled || (row.response.double_clicked() && !shown) {
            if open {
                st.tree_open.remove(&key);
            } else {
                st.tree_open.insert(key.clone());
            }
        } else if row.clicked {
            *act = Some(TreeAct::Open(Location::new(Source::Library, i, "")));
        }
        if !open {
            continue;
        }
        let state = if shown { f.cat.clone() } else { f.browser.cats.ensure(f.editor, (Source::Library, i)) };
        match state.cat {
            Some(cat) => {
                for &c in &cat.root().children {
                    if let Some(child) = cat.node(c) {
                        library_node(ui, st, f, &cat, child, 1, i, act);
                    }
                }
            }
            None if state.busy => busy_line(ui, "Reading the Library."),
            None => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn library_node(
    ui: &mut Ui,
    st: &mut Inner,
    f: &Frame<'_>,
    cat: &Catalog,
    node: &Node,
    depth: usize,
    kind: usize,
    act: &mut Option<TreeAct>,
) {
    let key = format!("library:{}:{}", kind, tree_key(&node.path));
    let here = st.loc.category == kind && st.loc.source == Source::Library;
    if here && st.scroll_to_selection && crate::model::is_ancestor(&node.path, &st.loc.folder) {
        st.tree_open.insert(key.clone());
    }
    let open = st.tree_open.contains(&key);
    let has_children = !node.children.is_empty();
    let id = Id::unique(("cb_tree_lib", f.host.index(), kind, tree_key(&node.path)));
    let selected = here && st.loc.folder.eq_ignore_ascii_case(&node.path);
    let row = local_tree_row(ui, id, depth, Tx::raw(node.name.as_str()), open, has_children, selected, false);
    if row.toggled || (row.response.double_clicked() && has_children) {
        if open {
            st.tree_open.remove(&key);
        } else {
            st.tree_open.insert(key.clone());
        }
    } else if row.clicked {
        *act = Some(TreeAct::Open(Location::new(Source::Library, kind, &node.path)));
    }
    // a reveal centres its folder in the tree, as it does its entry in the grid
    if selected && st.scroll_to_selection {
        row.response.scroll_to_me(Some(egui::Align::Center));
    }
    if open {
        for &c in &node.children {
            if let Some(child) = cat.node(c) {
                library_node(ui, st, f, cat, child, depth + 1, kind, act);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tree_node(
    ui: &mut Ui,
    st: &mut Inner,
    f: &Frame<'_>,
    cat: &Catalog,
    node: &Node,
    depth: usize,
    veils: Option<&FxHashSet<String>>,
    root_label: &str,
    act: &mut Option<TreeAct>,
) {
    let root = node.path.is_empty();
    let key = tree_key(&node.path);
    let has_children = !node.children.is_empty();
    // a reveal drops the browser into a folder this tree may have folded: the chain opens
    if st.scroll_to_selection && has_children && crate::model::is_ancestor(&node.path, &st.loc.folder) {
        st.tree_open.insert(key.clone());
    }
    let open = root || st.tree_open.contains(&key);
    let veiled = veils.is_some_and(|v| v.contains(&key));
    let label = if root { root_label } else { node.name.as_str() };
    let id = Id::unique(("cb_tree", f.host.index(), &key));
    let row = local_tree_row(
        ui,
        id,
        depth,
        Tx::raw(label),
        open,
        has_children && !root,
        st.loc.folder.eq_ignore_ascii_case(&node.path),
        veiled,
    );
    if row.toggled {
        if open {
            st.tree_open.remove(&key);
        } else {
            st.tree_open.insert(key.clone());
        }
    } else if row.clicked {
        *act = Some(TreeAct::Open(Location::new(Source::Project, 0, &node.path)));
    }
    if row.response.double_clicked() && has_children && !root {
        if open {
            st.tree_open.remove(&key);
        } else {
            st.tree_open.insert(key.clone());
        }
    }
    if !root && row.response.secondary_clicked() {
        let at = row.response.interact_pointer_pos().unwrap_or(row.response.rect.left_bottom());
        *act = Some(TreeAct::Menu(Entry::new(&node.path, true), at));
    }
    if veiled {
        row.response.clone().tip("Not shipped: mod.ltx [exclude] leaves it out of the build.");
    }
    // a project folder of the tree takes the browser's own tiles and files from Explorer
    let accepts = |p: &Payload| takes(st, p);
    if let Some(payload) = Zone::new(label, row.response.rect, &accepts).take(ui) {
        *act = Some(TreeAct::Drop(payload, node.path.to_string()));
    }
    // a reveal centres its folder in the tree, as it does its entry in the grid
    if st.scroll_to_selection && st.loc.folder.eq_ignore_ascii_case(&node.path) {
        row.response.scroll_to_me(Some(egui::Align::Center));
    }
    if open {
        for &c in &node.children {
            if let Some(child) = cat.node(c) {
                tree_node(ui, st, f, cat, child, if root { 0 } else { depth + 1 }, veils, root_label, act);
            }
        }
    }
}

/// What a project folder takes: the browser's own tiles (moved, or brought in from the Library)
/// and files from Explorer.
fn takes(st: &Inner, p: &Payload) -> bool {
    match p {
        Payload::Assets(_) => st.dragging.is_some(),
        Payload::Files(list) => !list.is_empty(),
        _ => false,
    }
}

/// A drop on the project folder `dst`.
fn take_drop(st: &mut Inner, f: &Frame<'_>, payload: Payload, dst: &str) {
    match payload {
        Payload::Assets(_) => {
            if let Some(from) = st.dragging.take() {
                actions::drop_into(f.editor, f.browser, &from, dst);
            }
        }
        Payload::Files(files) => {
            if !f.has_project {
                f.toast("Open a project to bring files into it.");
                return;
            }
            actions::import_files(f.editor, f.browser, files, dst);
        }
        _ => {}
    }
}

// ---- the tiles ------------------------------------------------------------------------------------

fn menu_id(host: Host) -> Id {
    Id::unique(("cb_entry_ctx", host.index()))
}

fn grid_menu_id(host: Host) -> Id {
    Id::unique(("cb_grid_ctx", host.index()))
}

/// What a tile asked for this frame; acted on once the grid is drawn.
#[derive(Default)]
struct GridOut {
    click: Option<(Entry, Modifiers)>,
    open: Option<Entry>,
    menu: Option<(Entry, Pos2)>,
    drop: Option<(Payload, String)>,
    drag: Option<Entry>,
    rename: Option<crate::local::RenameOut>,
    marquee: Vec<(Entry, Rect)>,
}

fn grid_body(ui: &mut Ui, cx: &mut DocCx<'_>, st: &mut Inner, f: &Frame<'_>, grid_rect: Rect) {
    // Ctrl+wheel sizes the tiles, and the list is the step under the smallest one
    let ctrl = f.ctx.input(|i| i.modifiers.ctrl);
    if ctrl && ui.rect_contains_pointer(grid_rect) {
        let wheel: f32 = f.ctx.input(|i| {
            i.events
                .iter()
                .filter_map(
                    |e| if let egui::Event::MouseWheel { delta, .. } = e { Some(delta.y) } else { None },
                )
                .sum()
        });
        if wheel != 0.0 {
            let host = f.host;
            let size = st.host(host).shown_tile(host);
            let v = st.host_mut(host);
            if v.list {
                if wheel > 0.0 {
                    v.list = false;
                    v.tile = gz_app::contract::TILE_MIN;
                }
            } else if wheel < 0.0 && size <= gz_app::contract::TILE_MIN {
                v.list = true;
            } else {
                v.tile = (size + wheel.signum() * 8.0)
                    .clamp(gz_app::contract::TILE_MIN, gz_app::contract::TILE_MAX);
            }
            st.view_dirty = true;
        }
    }

    let src = st.loc.source;
    if src == Source::Project && !f.has_project {
        empty_state(ui, Icon::Folder, "No project open", "", None);
        return;
    }
    // what a Library kind could not read is said over what it could
    if let Some(note) = f.cat.cat.as_ref().and_then(|c| c.note.clone()) {
        let unlinked = st.scope.1.is_empty() && crate::library::reads_game(st.loc.category);
        // what the Library's reader says, in its English
        if banner(ui, gz_ui::Tone::Info, Tx::ui(note.as_str()), unlinked.then(|| "Link Game...".into()), None)
            .is_some()
        {
            actions::link_game(f.editor);
        }
        ui.add_space(4.0);
    }
    let answer = f.browser.view(f.editor, &st.loc, &st.search, &f.cat);
    let Some(view) = answer.value().cloned() else {
        st.reveal_waits =
            st.scroll_to_selection && (f.cat.busy || matches!(answer, gz_query::Answer::Pending));
        if f.cat.busy || st.loc.source == Source::Library {
            busy_line(
                ui,
                if st.loc.source == Source::Library { "Reading the Library." } else { "Reading the folder." },
            );
        }
        return;
    };
    {
        let h = st.host_mut(f.host);
        h.entries = view.entries.len();
    }
    let bg = ui.interact(grid_rect, Id::unique(("cb_grid_bg", f.host.index())), Sense::click_and_drag());
    if view.entries.is_empty() {
        if search_active(&st.search) {
            empty_state(
                ui,
                Icon::Search,
                "No matches",
                "Nothing in this folder or below matches the search.",
                None,
            );
        } else {
            empty_state(
                ui,
                Icon::FolderOpen,
                "Empty folder",
                if src == Source::Project {
                    "Use Add to create a folder or an asset here, or drop files from Explorer."
                } else {
                    ""
                },
                None,
            );
        }
        grid_drop(ui, st, f, grid_rect);
        grid_menu(ui, st, f, &bg, grid_rect);
        return;
    }
    let mut out = GridOut::default();
    tiles(ui, st, f, &view, &mut out);

    // acted on once the grid is drawn: it may change what the grid lists
    if let Some((e, m)) = out.click.take() {
        st.focused = true;
        if m.shift && st.sel.anchor().is_some() {
            st.sel.range(&view.order(), &e);
        } else if m.ctrl || m.command {
            st.sel.toggle(e);
        } else {
            st.sel.only(e);
        }
    }
    if let Some((e, at)) = out.menu.take() {
        if !st.sel.contains(&e) {
            st.sel.only(e.clone());
        }
        st.menu = Some((Some(e), at));
        Popup::open_id(&f.ctx, menu_id(f.host));
    }
    if let Some(r) = out.rename {
        if r.cancelled {
            st.rename = None;
        } else if r.done {
            actions::commit_rename(f.editor, f.browser, st);
        }
    }
    if let Some(e) = out.drag {
        if !st.sel.contains(&e) {
            st.sel.only(e.clone());
        }
        st.drag_started = true;
    }
    if let Some((payload, dst)) = out.drop {
        take_drop(st, f, payload, &dst);
    }
    if let Some(e) = out.open.take() {
        actions::open_entry(cx, st, f.browser, &e, f.has_project);
    }
    st.drawn = std::mem::take(&mut out.marquee);

    // clicking empty space clears the selection; a drag there is a rubber band
    if bg.clicked() && !ctrl {
        st.sel.clear();
        st.focused = true;
    }
    marquee(ui, st, &bg);
    grid_drop(ui, st, f, grid_rect);
    grid_menu(ui, st, f, &bg, grid_rect);
}

/// The folder shown takes what lands on the grid outside a folder tile: the browser's own tiles
/// and files from Explorer, in the project only.
fn grid_drop(ui: &Ui, st: &mut Inner, f: &Frame<'_>, grid_rect: Rect) {
    if st.loc.source != Source::Project || !f.has_project {
        return;
    }
    let folder = st.loc.folder.clone();
    let taken = {
        let accepts = |p: &Payload| takes(st, p);
        Zone::new("Content Browser", grid_rect, &accepts).catch_all().take(ui)
    };
    if let Some(payload) = taken {
        // a tile dropped back into its own folder stays where it is
        if matches!(payload, Payload::Assets(_))
            && st.dragging.as_ref().is_some_and(|d| {
                d.key.0 == Source::Project
                    && d.entries
                        .iter()
                        .all(|e| crate::model::parent_of(&e.path).eq_ignore_ascii_case(&folder))
            })
        {
            st.dragging = None;
            return;
        }
        take_drop(st, f, payload, &folder);
    }
}

fn marquee(ui: &Ui, st: &mut Inner, bg: &egui::Response) {
    if bg.drag_started()
        && let Some(at) = bg.interact_pointer_pos()
    {
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        st.marquee = Some(crate::service::Marquee {
            start: at,
            base: if ctrl { st.sel.entries().to_vec() } else { Vec::new() },
        });
    }
    let Some(m) = st.marquee.clone() else { return };
    let Some(pointer) = ui.ctx().pointer_latest_pos() else { return };
    let band = Rect::from_two_pos(m.start, pointer);
    if band.width() > 3.0 || band.height() > 3.0 {
        let mut picked = m.base.clone();
        for (e, r) in &st.drawn {
            if r.intersects(band) && !picked.contains(e) {
                picked.push(e.clone());
            }
        }
        st.sel.set(picked);
        local_marquee(ui.painter(), m.start, pointer);
    }
    if bg.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
        st.marquee = None;
    }
}

/// The geometry of the grid for this frame.
struct Layout {
    list: bool,
    per_row: usize,
    cell_w: f32,
    cell_h: f32,
    row_h: f32,
    tile_w: f32,
    caption_font: egui::FontId,
    caption_line: f32,
}

fn layout(st: &Inner, f: &Frame<'_>, avail: f32) -> Layout {
    let shown = st.host(f.host).shown_tile(f.host);
    // a tile too small for a folder's name is a row of the list
    let list = st.host(f.host).list || shown < LIST_BELOW;
    let tile_w = shown.min(avail).max(24.0);
    if list {
        let col = units::em(LIST_COLUMN_EM);
        let per_row = (((avail + TILE_GAP) / (col + TILE_GAP)) as usize).max(1);
        let cell_w = ((avail - TILE_GAP * (per_row - 1) as f32) / per_row as f32).floor().max(1.0);
        Layout {
            list,
            per_row,
            cell_w,
            cell_h: LIST_ROW,
            row_h: LIST_ROW,
            tile_w,
            caption_font: Font::Body.id(),
            caption_line: Font::Body.line(),
        }
    } else {
        let per_row = (((avail + TILE_GAP) / (tile_w + TILE_GAP)) as usize).max(1);
        let caption_line = Font::Small.line();
        let cell_h = tile_w + caption_line * 2.0 + TILE_PAD;
        Layout {
            list,
            per_row,
            cell_w: tile_w,
            cell_h,
            row_h: cell_h + TILE_GAP,
            tile_w,
            caption_font: Font::Small.id(),
            caption_line,
        }
    }
}

/// The scroll that shows rows `top..bottom` (Unreal's rules): on screen stays put, just past an
/// edge pulls in with half a row to spare, far away is centred.
fn scroll_into_view(scroll: f32, view_h: f32, top: f32, bottom: f32) -> Option<f32> {
    let h = bottom - top;
    if top >= scroll && bottom <= scroll + view_h {
        return None;
    }
    Some(if top < scroll && top >= scroll - 2.0 * h {
        top - h * 0.5
    } else if bottom > scroll + view_h && bottom <= scroll + view_h + 2.0 * h {
        bottom - view_h + h * 0.5
    } else {
        top + h * 0.5 - view_h * 0.5
    })
}

fn tiles(ui: &mut Ui, st: &mut Inner, f: &Frame<'_>, view: &Arc<View>, out: &mut GridOut) {
    let avail = ui.available_width().max(1.0);
    st.host_mut(f.host).grid_w = avail;
    let lay = layout(st, f, avail);
    let total = view.entries.len();
    let rows = total.div_ceil(lay.per_row);
    let shown = (st.loc.source, st.loc.category, st.loc.folder.to_lowercase());
    let mut offset: Option<f32> = None;
    // a new folder starts at its top
    if st.shown_folder.as_ref() != Some(&shown) {
        st.shown_folder = Some(shown);
        offset = Some(0.0);
    }
    // a selection made from outside lands on screen: the last selected tile in grid order
    let reveal = if st.scroll_to_selection {
        view.entries.iter().rposition(|e| st.sel.contains(&e.entry))
    } else {
        st.rename.as_ref().filter(|r| r.focus).and_then(|r| view.find(&r.entry))
    };
    if let Some(i) = reveal {
        let row = (i / lay.per_row) as f32;
        let hv = st.host(f.host);
        offset = scroll_into_view(hv.scroll, hv.viewport, row * lay.row_h, row * lay.row_h + lay.cell_h)
            .or(offset);
    }
    let ctrl = f.ctx.input(|i| i.modifiers.ctrl);
    let mut area = ScrollArea::vertical()
        .id_salt(("cb_tiles", f.host.index()))
        .auto_shrink([false, false])
        .scroll_source(egui::containers::scroll_area::ScrollSource {
            mouse_wheel: !ctrl,
            ..egui::containers::scroll_area::ScrollSource::ALL
        });
    if let Some(o) = offset {
        area = area.vertical_scroll_offset(o.max(0.0));
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let focused = st.focused;
    let result = area.show_rows(ui, lay.row_h, rows, |ui, range| {
        let mut rename_at: Option<(Rect, Entry)> = None;
        for row in range {
            let (row_rect, _) = ui.allocate_exact_size(Vec2::new(avail, lay.row_h), Sense::hover());
            for col in 0..lay.per_row {
                let i = row * lay.per_row + col;
                let Some(ve) = view.entries.get(i) else { break };
                let a = pos2(row_rect.min.x + col as f32 * (lay.cell_w + TILE_GAP), row_rect.min.y);
                let cell = Rect::from_min_size(a, Vec2::new(lay.cell_w, lay.cell_h));
                if let Some(r) = tile(ui, st, f, &lay, ve, cell, focused, out) {
                    rename_at = Some((r, ve.entry.clone()));
                }
                out.marquee.push((ve.entry.clone(), cell));
            }
        }
        // the inline rename box goes over the grid, up to 24 em wide, drawn last to sit above
        // the neighbouring tiles
        if let Some((at, entry)) = rename_at
            && let Some(r) = st.rename.as_mut().filter(|r| r.entry == entry)
        {
            let want = text::width(ui.ctx(), &Font::Body.id(), Tx::raw(r.text.as_str())) + 12.0 + 12.0;
            let w = want.clamp(at.width(), at.width().max(units::em(24.0))).min(avail);
            let x = if lay.list { at.min.x } else { at.center().x - w * 0.5 };
            let left = ui.max_rect().min.x;
            let x = x.clamp(left, (left + avail - w).max(left));
            let rect = Rect::from_min_size(pos2(x, at.min.y), Vec2::new(w, metrics::FRAME));
            let id = Id::unique(("cb_rename", f.host.index()));
            let res = local_rename_box(ui, id, rect, &mut r.text, r.select, r.focus);
            r.focus = false;
            out.rename = Some(res);
        }
    });
    let hv = st.host_mut(f.host);
    hv.scroll = result.state.clamped_offset().y;
    hv.viewport = result.inner_rect.height();
}

/// Draws one tile or list row; answers where its rename box goes when it has one.
#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut Ui,
    st: &mut Inner,
    f: &Frame<'_>,
    lay: &Layout,
    ve: &ViewEntry,
    cell: Rect,
    focused: bool,
    out: &mut GridOut,
) -> Option<Rect> {
    let p = &f.p;
    let path = ve.entry.path.as_str();
    let leaf = leaf_of(path);
    // the editor's own assets go by their name alone: the icon already says the kind. So does
    // an entry of a Library kind: the kind lists one extension and its name is on the breadcrumb
    let own_ext = if ve.entry.folder {
        None
    } else {
        gz_project::own_extension(leaf).or_else(|| {
            let ext = st.loc.lib_kind().map(|k| k.ext).filter(|e| !e.is_empty())?;
            let cut = leaf.len().checked_sub(ext.len())?;
            (cut > 0 && leaf.is_char_boundary(cut) && leaf[cut..].eq_ignore_ascii_case(ext)).then_some(ext)
        })
    };
    // a spawn section goes by the name the player reads; a kind of the Library's top by the
    // editor's word for it
    let kind_word = st.loc.home().then(|| Tx::ui(leaf).text(&f.ctx));
    let name = kind_word.as_deref().unwrap_or_else(|| {
        ve.label.as_deref().unwrap_or_else(|| own_ext.map_or(leaf, |e| &leaf[..leaf.len() - e.len()]))
    });
    let selected = st.sel.contains(&ve.entry);
    let renaming = st.rename.as_ref().is_some_and(|r| r.entry == ve.entry);
    let veiled = ve.ship.veiled();
    let id = Id::unique(("cb_tile", f.host.index(), path, ve.entry.folder));
    let resp = ui.interact(cell, id, Sense::click_and_drag());
    let visible = ui.is_rect_visible(cell);

    // the picture's square: a tile's own, or the one a list row starts with
    let square = if lay.list {
        Rect::from_min_size(
            pos2((cell.min.x + 3.0).round(), (cell.min.y + (cell.height() - LIST_PICTURE) * 0.5).round()),
            Vec2::splat(LIST_PICTURE),
        )
    } else {
        Rect::from_min_max(
            cell.min + Vec2::splat(TILE_PAD),
            pos2(cell.max.x - TILE_PAD, cell.min.y + lay.tile_w - TILE_PAD),
        )
    };
    let mut shown: Option<Shown> = None;
    if visible && ve.picture != Picture::None {
        let ppp = f.ctx.pixels_per_point();
        let key: PicKey = (st.loc.source, st.loc.category, path.to_lowercase());
        let cat = f.cat.cat.clone();
        let rel = path.to_owned();
        shown = st.pics.get(&key, ve.picture, path, square.width() * ppp, f.now, move || {
            let cat = cat?;
            let file = cat.file(&rel)?;
            // a media asset's picture is made from what it describes
            if file.asset.is_some_and(|k| k.media().is_some()) {
                return Some(Bytes::Asset(cat.disk_root.clone()?, rel.clone()));
            }
            match file.origin {
                Origin::Game => Some(Bytes::Game(cat.game.clone()?, cat.game_path(&rel)?)),
                Origin::Disk => cat.disk_path(&rel).map(Bytes::Disk),
                Origin::None => None,
            }
        });
    }
    if visible {
        let painter = ui.painter();
        let r = CornerRadius::same(metrics::RADIUS);
        if selected {
            painter.rect_filled(cell, r, if focused { p.selection_focused } else { p.selection });
        } else if resp.hovered() {
            painter.rect_filled(cell, r, p.hover);
        }
        if !selected && !lay.list {
            painter.rect_filled(square, CornerRadius::same(3), p.raised);
        }
        match (shown, ve.own) {
            (Some(s), _) => draw_picture(painter, s, square),
            (None, Some(kind)) => {
                let side = if lay.list { LIST_PICTURE } else { glyph_side(square) };
                icons::draw(painter, crate::icons::own_icon(kind), centred(square, side), p.text);
            }
            (None, None) => {
                let side = if lay.list { 16.0 } else { glyph_side(square) };
                if side >= 4.0 {
                    icons::draw(painter, ve.glyph, centred(square, side), p.text_secondary);
                }
            }
        }
        // over the picture, under the outline: a left-out tile still reads as selected
        if veiled {
            painter.rect_filled(square, CornerRadius::same(3), p.inset.gamma_multiply(0.6));
        }
        // a Library asset a scene uses and the project lacks: a dot in the warning tone
        if ve.wanted {
            let r = if lay.list { 3.0 } else { 4.5 };
            let c = pos2(square.max.x - r - 2.0, square.min.y + r + 2.0);
            painter.circle_filled(c, r, p.warning);
        }
        if selected && focused {
            painter.rect_stroke(cell, r, Stroke::new(1.5, p.accent_bright), egui::StrokeKind::Inside);
        }
    }
    let text_x = if lay.list { square.max.x + 6.0 } else { cell.min.x + TILE_PAD };
    let caption_y = if lay.list {
        (cell.min.y + (cell.height() - lay.caption_line) * 0.5).round()
    } else {
        cell.min.y + lay.tile_w
    };
    let rename_rect = renaming.then(|| {
        if lay.list {
            Rect::from_min_max(pos2(text_x, cell.min.y), cell.max)
        } else {
            Rect::from_min_size(pos2(cell.min.x, caption_y), Vec2::new(lay.tile_w, metrics::FRAME))
        }
    });
    if visible && !renaming {
        let width = if lay.list { (cell.max.x - 4.0 - text_x).max(1.0) } else { lay.tile_w - TILE_PAD * 2.0 };
        let font = lay.caption_font.clone();
        let ctx = f.ctx.clone();
        let measure = |s: &str| text::width(&ctx, &font, Tx::raw(s));
        let cap = caption::caption(
            name,
            !ve.entry.folder && own_ext.is_none() && ve.label.is_none(),
            if lay.list { 1 } else { 2 },
            width,
            &measure,
        );
        let color = if veiled { p.text_muted } else { p.text };
        let painter = ui.painter();
        for (n, line) in cap.lines.iter().enumerate() {
            let y = caption_y + n as f32 * lay.caption_line;
            let (pos, anchor) = if lay.list {
                (pos2(text_x, y + lay.caption_line * 0.5), Align2::LEFT_CENTER)
            } else {
                (pos2(cell.min.x + TILE_PAD + width * 0.5, y + lay.caption_line * 0.5), Align2::CENTER_CENTER)
            };
            painter.text(pos, anchor, line, lay.caption_font.clone(), color);
        }
        if cap.truncated
            && let Some(last) = cap.lines.last()
        {
            let n = cap.lines.len() - 1;
            let r = Rect::from_min_size(
                pos2(text_x, caption_y + n as f32 * lay.caption_line),
                Vec2::new(width, lay.caption_line),
            );
            gz_ui::audit::truncated(&f.ctx, name, last, r);
        }
    }

    // the menu MCP's ui.popup puts up, the way a right click does, at the entry
    if visible && take_popup_request(&f.ctx, &format!("cb_context:{path}")) {
        out.menu = Some((ve.entry.clone(), cell.right_bottom()));
    }
    if resp.clicked() {
        let m = f.ctx.input(|i| i.modifiers);
        out.click = Some((ve.entry.clone(), m));
    }
    if resp.double_clicked() {
        out.open = Some(ve.entry.clone());
    }
    if resp.secondary_clicked() {
        out.menu = Some((ve.entry.clone(), resp.interact_pointer_pos().unwrap_or(cell.right_bottom())));
    }
    // a drag carries the selection when the tile is part of it: the kit's payload for whoever
    // takes it, the entries for the browser's own folders
    if (resp.drag_started() || (resp.dragged() && st.dragging.is_some())) && !renaming && !st.loc.home() {
        if resp.drag_started() || st.drag_payload.is_none() {
            let entries: Vec<Entry> =
                if selected { st.sel.entries().to_vec() } else { vec![ve.entry.clone()] };
            if let Some(cat) = f.cat.cat.clone() {
                let assets = entries.iter().map(|e| crate::payload_of(&cat, &e.path, e.folder)).collect();
                st.drag_payload = Some(Payload::Assets(assets));
            }
            st.dragging = Some(Dragging { key: cat_key(&st.loc), entries: entries.clone() });
            out.drag = Some(ve.entry.clone());
            st.drag_icon = Some((ve.glyph, ve.own, shown, name.to_owned(), entries.len()));
        }
        if let Some(payload) = st.drag_payload.clone() {
            gz_ui::drop::start(&resp, payload);
        }
    }
    // a project folder takes a drop
    if ve.entry.folder && st.loc.source == Source::Project && visible {
        let accepts = |p: &Payload| takes(st, p);
        if let Some(payload) = Zone::new(name, cell, &accepts).take(ui) {
            out.drop = Some((payload, path.to_owned()));
        }
    }
    if resp.hovered() && !renaming && !resp.dragged() {
        // one line each, every line in the window's language but the path
        let mut lines: Vec<Tx<'static>> = vec![Tx::raw(path.to_owned())];
        if let Some(s) = shown
            && let Some((w, h)) = s.source_size
        {
            lines.push(match s.alpha {
                TileAlpha::Opaque => Tx::raw(format!("{w} x {h}")),
                TileAlpha::Cutout => tr!("{w} x {h}, fully transparent", w = w, h = h),
                TileAlpha::Translucent => tr!("{w} x {h}, partly transparent", w = w, h = h),
            });
        }
        if let Some(note) = &ve.note {
            // a note the Library's reader wrote, in its English
            lines.push(Tx::ui(note.to_string()));
        }
        if !ve.entry.folder && crate::icons::extension(path).is_some_and(|e| e.eq_ignore_ascii_case(".level"))
        {
            lines.push("An X-Ray SDK scene: it is imported into a .scene when it opens.".into());
        }
        if ve.wanted {
            lines.push(
                "A scene of the project uses it, and the project does not hold it: Bring into Project copies it in."
                    .into(),
            );
        }
        if let Some(kind) = ve.own {
            lines.push(Tx::from(crate::icons::own_name(kind)));
        }
        // a media asset goes by the name the game knows it by
        if let Some(name) = f.cat.cat.as_ref().and_then(|c| c.file(path)).and_then(|f| f.published.clone()) {
            lines.push(tr!("The game knows it as {name}.", name = name.to_string()));
        }
        match &ve.ship {
            crate::catalog::Ship::Excluded(rule) => {
                lines.push(tr!(
                    "Not shipped: excluded by '{rule}' in mod.ltx [exclude].",
                    rule = rule.as_str()
                ));
            }
            crate::catalog::Ship::Never if st.loc.source == Source::Project => {
                lines.push("A source: never shipped.".into());
            }
            _ => {}
        }
        let tip: Vec<String> = lines.iter().map(|l| l.text(&f.ctx)).collect();
        resp.tip(Tx::raw(tip.join("\n")));
    }
    rename_rect
}

/// A glyph the way the own icons sit in a tile: 58% of the square, stopping at 48 pt.
fn glyph_side(square: Rect) -> f32 {
    (square.width().min(square.height()) * 0.58 * 16.0 / 14.0).min(48.0)
}

fn centred(rect: Rect, side: f32) -> Rect {
    Rect::from_min_size((rect.center() - Vec2::splat(side * 0.5)).round(), Vec2::splat(side))
}

/// A picture fitted into the square, never stretched, transparency on the checkerboard.
fn draw_picture(painter: &egui::Painter, s: Shown, square: Rect) {
    let (w, h) = (square.width(), square.height());
    if w < 1.0 || h < 1.0 || s.aspect <= 0.0 {
        return;
    }
    let by_height = w / h > s.aspect;
    let (fw, fh) = if by_height { (h * s.aspect, h) } else { (w, w / s.aspect) };
    let rect = Rect::from_center_size(square.center(), Vec2::new(fw.round().max(1.0), fh.round().max(1.0)));
    if s.alpha != TileAlpha::Opaque {
        // cells of 8 pt, down to 4 in a small picture
        let cell = (rect.width().max(rect.height()) / 8.0).clamp(4.0, 8.0).round();
        gz_ui::checker(painter, rect.min, rect, cell);
    }
    painter.image(s.tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
}

/// What follows the pointer while tiles are dragged: the picture or glyph, the name, and how
/// many more go with it.
fn drag_preview(ctx: &egui::Context, st: &mut Inner, f: &Frame<'_>) {
    if gz_ui::drop::dragged(ctx).is_none() {
        st.drag_icon = None;
        st.drag_started = false;
        st.drag_payload = None;
        // what the browser dragged was let go: a drop on one of its folders took it already
        if !ctx.input(|i| i.pointer.any_down()) {
            st.dragging = None;
        }
        return;
    }
    let Some((glyph, own, shown, name, n)) = st.drag_icon.clone() else { return };
    let Some(at) = ctx.pointer_latest_pos() else { return };
    egui::Area::new(Id::unique(("cb_drag", f.host.index())))
        .order(egui::Order::Tooltip)
        .fixed_pos(at + Vec2::new(12.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            gz_ui::menu::popup_frame(ctx).show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                match (shown, own) {
                    (Some(s), _) => draw_picture(ui.painter(), s, rect),
                    (None, Some(k)) => {
                        icons::draw(ui.painter(), crate::icons::own_icon(k), centred(rect, 40.0), f.p.text)
                    }
                    (None, None) => icons::draw(ui.painter(), glyph, centred(rect, 32.0), f.p.text_secondary),
                }
                text::label(ui, Tx::raw(name.as_str()), Font::Body, f.p.text, Clip::Middle);
                if n > 1 {
                    text::label(
                        ui,
                        tr!("and {n} more", n = n - 1),
                        Font::Small,
                        f.p.text_secondary,
                        Clip::End,
                    );
                }
            });
        });
}

// ---- the menus ------------------------------------------------------------------------------------

/// The right click on empty grid space.
fn grid_menu(ui: &Ui, st: &mut Inner, f: &Frame<'_>, bg: &egui::Response, grid_rect: Rect) {
    let id = grid_menu_id(f.host);
    if bg.secondary_clicked() {
        st.menu = None;
        Popup::close_id(&f.ctx, menu_id(f.host));
        st.grid_menu_at = bg.interact_pointer_pos().unwrap_or(grid_rect.min);
        Popup::open_id(&f.ctx, id);
    }
    if take_popup_request(&f.ctx, "cb_grid") {
        st.grid_menu_at = grid_rect.min + Vec2::splat(16.0);
        Popup::open_id(&f.ctx, id);
    }
    if !Popup::is_id_open(&f.ctx, id) {
        return;
    }
    let anchor = Rect::from_min_size(st.grid_menu_at, Vec2::ZERO);
    let mut add_pick: Option<usize> = None;
    let mut paste = false;
    let mut all = false;
    let mut clear = false;
    let mut refresh_ = false;
    let blocked = actions::paste_blocked(st);
    let has_sel = !st.sel.is_empty();
    let project = st.loc.source == Source::Project;
    let add_off = st.add_blocked(f.has_project);
    menu_popup(&f.ctx, id, anchor, ui.layer_id(), |ui| {
        // authoring lives here too, on the empty space of the writable source
        if project {
            add_menu(ui, add_off, &mut add_pick);
            ui.separator();
        }
        paste = menu_item(
            ui,
            "Paste",
            MenuItem {
                keys: "Ctrl+V",
                enabled: blocked.is_none(),
                tip: blocked.unwrap_or("").into(),
                ..MenuItem::default()
            },
        );
        ui.separator();
        all = menu_item(ui, "Select All", MenuItem { keys: "Ctrl+A", ..MenuItem::default() });
        clear = menu_item(ui, "Clear Selection", MenuItem { enabled: has_sel, ..MenuItem::default() });
        refresh_ = menu_item(ui, "Refresh", MenuItem { keys: "F5", ..MenuItem::default() });
    });
    if let Some(i) = add_pick {
        run_add(st, f, i);
    }
    if paste {
        actions::paste(&f.ctx, f.editor, f.browser, st);
    }
    if all {
        select_all(st, f);
    }
    if clear {
        st.sel.clear();
    }
    if refresh_ {
        refresh(st, f);
    }
}

fn select_all(st: &mut Inner, f: &Frame<'_>) {
    if let Some(view) = f.browser.view(f.editor, &st.loc, &st.search, &f.cat).value() {
        st.sel.all(&view.order());
    }
}

/// The menu of an entry: one for folders and files alike.
fn entry_menu(st: &mut Inner, f: &Frame<'_>, layer: egui::LayerId) {
    let id = menu_id(f.host);
    let Some((Some(entry), at)) = st.menu.clone() else { return };
    if !Popup::is_id_open(&f.ctx, id) {
        st.menu = None;
        return;
    }
    let count = st.sel.len();
    let writable = st.loc.source == Source::Project;
    let library = st.loc.source == Source::Library && !st.loc.home();
    let can_paste = actions::paste_blocked(st).is_none();
    let shown_cat = f.cat.cat.clone();
    let place_ref = if entry.folder {
        None
    } else {
        shown_cat.as_deref().and_then(|c| actions::placeable(c, &entry.path))
    };
    let scene_open = f.editor.project_opt().is_some();
    let place_why = if place_ref.is_none() {
        "Only objects, groups, sounds, particles and spawn sections are placed in a level."
    } else if !scene_open {
        "Open a project to place it in its level."
    } else {
        ""
    };
    let bring_why = match shown_cat.as_deref().and_then(|c| c.file(&entry.path)).map(|f| f.origin) {
        _ if !f.has_project => "Open a project to bring it in.",
        Some(Origin::None) => "It has no file of its own: place it in a level instead.",
        _ => "",
    };
    let (rules, _) = ContentBrowser::rules(f.editor);
    let toggle = (writable && f.has_project).then(|| actions::build_toggle(&rules, st.sel.entries()));
    let head =
        if count > 1 { tr!("{n} selected", n = count) } else { Tx::raw(leaf_of(&entry.path).to_owned()) };
    #[derive(PartialEq)]
    enum Pick {
        Open,
        Place,
        Bring,
        Copy,
        Cut,
        Paste,
        Rename,
        Delete,
        Exclude,
        Include,
        CopyPath,
        Reveal,
    }
    let mut pick: Option<Pick> = None;
    let p = f.p.clone();
    menu_popup(&f.ctx, id, Rect::from_min_size(at, Vec2::ZERO), layer, |ui| {
        text::label(ui, &head, Font::Body, p.text_muted, Clip::End);
        ui.separator();
        let mut row = |ui: &mut Ui, label: &'static str, item: MenuItem<'_>, what: Pick| {
            if menu_item(ui, label, item) {
                pick = Some(what);
            }
        };
        row(ui, "Open", MenuItem { icon: Icon::Open, keys: "Enter", ..MenuItem::default() }, Pick::Open);
        if !entry.folder {
            row(
                ui,
                "Place in Level",
                MenuItem {
                    icon: Icon::Place,
                    enabled: place_why.is_empty(),
                    tip: place_why.into(),
                    ..MenuItem::default()
                },
                Pick::Place,
            );
        }
        if library {
            row(
                ui,
                "Bring into Project",
                MenuItem {
                    icon: Icon::New,
                    enabled: bring_why.is_empty(),
                    tip: bring_why.into(),
                    ..MenuItem::default()
                },
                Pick::Bring,
            );
        }
        ui.separator();
        row(ui, "Copy", MenuItem { icon: Icon::Copy, keys: "Ctrl+C", ..MenuItem::default() }, Pick::Copy);
        row(
            ui,
            "Cut",
            MenuItem { icon: Icon::Cut, keys: "Ctrl+X", enabled: writable, ..MenuItem::default() },
            Pick::Cut,
        );
        row(
            ui,
            "Paste",
            MenuItem { icon: Icon::Paste, keys: "Ctrl+V", enabled: can_paste, ..MenuItem::default() },
            Pick::Paste,
        );
        row(
            ui,
            "Rename",
            MenuItem {
                icon: Icon::Rename,
                keys: "F2",
                enabled: writable && count == 1,
                ..MenuItem::default()
            },
            Pick::Rename,
        );
        // the item that asks is quiet, in the error colour; the loud button is the confirmation's
        row(
            ui,
            "Delete...",
            MenuItem {
                icon: Icon::Delete,
                keys: "Del",
                enabled: writable,
                danger: true,
                ..MenuItem::default()
            },
            Pick::Delete,
        );
        if let Some(t) = &toggle {
            ui.separator();
            match t {
                BuildToggle::Never => row(
                    ui,
                    "Exclude from Build",
                    MenuItem {
                        icon: Icon::Exclude,
                        enabled: false,
                        tip: "The editor's own files never ship: nothing here goes into a build.".into(),
                        ..MenuItem::default()
                    },
                    Pick::Exclude,
                ),
                BuildToggle::Exclude(_) => row(
                    ui,
                    "Exclude from Build",
                    MenuItem { icon: Icon::Exclude, ..MenuItem::default() },
                    Pick::Exclude,
                ),
                BuildToggle::Include(_) => row(
                    ui,
                    "Include in Build",
                    MenuItem { icon: Icon::Exclude, ..MenuItem::default() },
                    Pick::Include,
                ),
                BuildToggle::ByRule(rule) => {
                    let tip = tr!(
                        "The rule {rule} of mod.ltx [exclude] leaves it out: include that instead.",
                        rule = rule.as_str()
                    );
                    row(
                        ui,
                        "Include in Build",
                        MenuItem { icon: Icon::Exclude, enabled: false, tip, ..MenuItem::default() },
                        Pick::Include,
                    )
                }
            }
        }
        ui.separator();
        row(ui, "Copy Path", MenuItem { icon: Icon::Link, ..MenuItem::default() }, Pick::CopyPath);
        if writable {
            row(ui, "Show in Explorer", MenuItem { icon: Icon::Reveal, ..MenuItem::default() }, Pick::Reveal);
        }
    });
    let Some(pick) = pick else { return };
    st.menu = None;
    let cat = f.browser.cats.peek(cat_key(&st.loc));
    match pick {
        Pick::Open => st.pending_open = Some(entry),
        Pick::Place => {
            if let (Some(r), Some(c)) = (place_ref, cat.as_ref()) {
                actions::place(f.editor, crate::payload_of(c, &entry.path, false), r);
            }
        }
        Pick::Bring => {
            if let Some(cat) = cat.clone() {
                let entries = st.sel.entries().to_vec();
                let ed = f.editor.clone();
                f.editor.exec.spawn(async move {
                    let (done, errors) = actions::bring_into_project(&ed, &cat, &entries).await;
                    match errors.first() {
                        None => ed.log.info(format!("Brought {done} asset(s) into the project.")),
                        Some(e) => ed.notices.error(format!(
                            "Brought {done} asset(s) into the project; {} could not come: {e}",
                            errors.len()
                        )),
                    }
                    ed.exec.wake().ping();
                });
            }
        }
        Pick::Copy => actions::clipboard_copy(&f.ctx, f.editor, st, cat.as_deref(), false),
        Pick::Cut => actions::clipboard_copy(&f.ctx, f.editor, st, cat.as_deref(), true),
        Pick::Paste => actions::paste(&f.ctx, f.editor, f.browser, st),
        Pick::Rename => actions::begin_rename(st, &entry),
        Pick::Delete => actions::delete_selection(&f.ctx, f.editor, f.browser, st, f.has_project),
        Pick::Exclude | Pick::Include => match toggle {
            Some(BuildToggle::Exclude(paths)) => actions::set_excluded(f.editor, paths, true),
            Some(BuildToggle::Include(paths)) => actions::set_excluded(f.editor, paths, false),
            _ => {}
        },
        Pick::CopyPath => f.ctx.copy_text(entry.path.to_string()),
        Pick::Reveal => {
            let root = f.editor.project_root();
            let path = entry.path.to_string();
            let ed = f.editor.clone();
            f.editor.exec.spawn_io(move || {
                if let Err(e) = actions::reveal_in_explorer(root, &path, false) {
                    ed.notices.error(e);
                }
            });
        }
    }
}

// ---- the status strip and the keys --------------------------------------------------------------

fn status_strip(ui: &mut Ui, st: &Inner, f: &Frame<'_>) {
    let h = 24.0;
    let w = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let painter = ui.painter();
    painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, f.p.border_subtle));
    // the folder shown - or what the search found in it - never the whole source
    let (folders, files) = f
        .browser
        .view(f.editor, &st.loc, &st.search, &f.cat)
        .value()
        .map_or((0, 0), |v| (v.folders, v.files));
    let shown = folders + files;
    let searching = search_active(&st.search);
    // the counts with their words in the window's language, the number written with its commas
    let mut parts: Vec<Tx<'static>> = vec![if searching {
        tr!("{count} {n:result|results}", count = thousands(shown), n = shown)
    } else {
        tr!("{count} {n:item|items}", count = thousands(shown), n = shown)
    }];
    let selected = st.selected_tiles();
    if selected > 0 {
        parts.push(tr!("{count} selected", count = thousands(selected)));
    }
    if !st.clip.items.is_empty() {
        let count = thousands(st.clip.items.len());
        parts.push(if st.clip.cut {
            tr!("{count} cut", count = count)
        } else {
            tr!("{count} on the clipboard", count = count)
        });
    }
    let line: Vec<String> = parts.iter().map(|t| t.text(&f.ctx)).collect();
    let line = line.join("  \u{b7}  ");
    let read_only = st.loc.source.read_only();
    let badge_w = if read_only { text::width(&f.ctx, &Font::Small.id(), "Read only") + 12.0 } else { 0.0 };
    text::paint(
        painter,
        &Font::Small.id(),
        pos2(rect.min.x + 2.0, rect.center().y),
        Align2::LEFT_CENTER,
        f.p.text_secondary,
        Tx::raw(line.as_str()),
        Some((w - badge_w - 14.0).max(0.0)),
        Clip::End,
    );
    if read_only {
        let r = Rect::from_min_max(pos2(rect.max.x - badge_w, rect.min.y), rect.max);
        let mut child = ui
            .new_child(UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)));
        badge(&mut child, "Read only", gz_ui::Tone::Muted);
        child
            .interact(r, Id::unique(("cb_read_only", f.host.index())), Sense::hover())
            .tip("Browse, preview and copy out into the project: nothing in this source is changed.");
    }
}

/// The browser's keys while it has the focus and no text field has them.
fn shortcuts(ctx: &egui::Context, cx: &mut DocCx<'_>, st: &mut Inner, f: &Frame<'_>, rect: Rect) {
    // the last click decides who owns the keys
    if ctx.input(|i| i.pointer.any_pressed())
        && let Some(at) = ctx.input(|i| i.pointer.interact_pos())
    {
        st.focused = rect.contains(at);
    }
    // the shell keeps the scene's bindings away while the browser has the keys
    if st.focused {
        cx.requests.own_keys(crate::PANEL_ID);
    }
    if let Some(e) = st.pending_open.take() {
        actions::open_entry(cx, st, f.browser, &e, f.has_project);
    }
    if !st.focused || ctx.egui_wants_keyboard_input() || st.rename.is_some() {
        return;
    }
    let take = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
    let before = st.loc.clone();
    if take(Modifiers::ALT, Key::ArrowLeft) {
        let r = st.go_back(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if take(Modifiers::ALT, Key::ArrowRight) {
        let r = st.go_forward(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if take(Modifiers::ALT, Key::ArrowUp) {
        let r = st.go_up(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if take(Modifiers::ALT, Key::Home) {
        let r = st.go_home(&f.browser.cats, f.has_project);
        nav_result(st, f, r, &before);
    }
    if take(Modifiers::NONE, Key::Delete) {
        actions::delete_selection(ctx, f.editor, f.browser, st, f.has_project);
    }
    if take(Modifiers::NONE, Key::F2)
        && st.sel.len() == 1
        && let Some(e) = st.sel.entries().first().cloned()
    {
        actions::begin_rename(st, &e);
    }
    if take(Modifiers::NONE, Key::Enter)
        && st.sel.len() == 1
        && let Some(e) = st.sel.entries().first().cloned()
    {
        actions::open_entry(cx, st, f.browser, &e, f.has_project);
    }
    if take(Modifiers::NONE, Key::F5) {
        refresh(st, f);
    }
    let cat = f.browser.cats.peek(cat_key(&st.loc));
    // egui hands Ctrl+C, Ctrl+X and Ctrl+V over as clipboard events
    let event = |want: fn(&egui::Event) -> bool| ctx.input(|i| i.events.iter().any(want));
    if take(Modifiers::COMMAND, Key::C) || event(|e| matches!(e, egui::Event::Copy)) {
        actions::clipboard_copy(ctx, f.editor, st, cat.as_deref(), false);
    }
    if take(Modifiers::COMMAND, Key::X) || event(|e| matches!(e, egui::Event::Cut)) {
        actions::clipboard_copy(ctx, f.editor, st, cat.as_deref(), true);
    }
    if take(Modifiers::COMMAND, Key::V) || event(|e| matches!(e, egui::Event::Paste(_))) {
        actions::paste(ctx, f.editor, f.browser, st);
    }
    if take(Modifiers::COMMAND, Key::A) {
        select_all(st, f);
    }
}
