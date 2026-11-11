//! The browser's commands. The wire is `LevelMcpContent.cpp`'s: the same argument names and
//! forgiving parsing, the same reply fields in the C++ order, the same error texts.
//!
//! A command awaits what it reads (the project's tree takes in what was written, a source is
//! listed when it was not), then changes the view state on the UI thread at the start of the
//! next frame (`on_ui`), and brings the browser into view through the shell's `ui_drawer`, which
//! raises the Level panel when it shows the browser and opens the Content Drawer otherwise.

use std::sync::Arc;

use gz_app::contract::{TILE_MAX, TILE_MIN, TILE_RANGE_ERROR};
use gz_app::on_ui;
use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Reply, Value, json};
use gz_project::Project;
use serde_json::Map;

use crate::actions;
use crate::icons;
use crate::model::{CATEGORIES, Host, Location, Source, find_category, normalize_name};
use crate::select::Entry;
use crate::service::{CatKey, ContentBrowser, Inner, cat_key, category_name};

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

const SOURCE_ERROR: &str = "source must be project, editor, darf or 0/1/2";
const CATEGORY_ERROR: &str = "unknown category (see list_assets)";

/// `source`: a name or a number 0..2; absent is the current one.
fn parse_source(args: &Args) -> Result<Option<Source>, CmdError> {
    match args.get("source") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Source::parse_name(s).map(Some).ok_or_else(|| refuse(SOURCE_ERROR)),
        Some(Value::Number(n)) => {
            n.as_i64().and_then(Source::from_index).map(Some).ok_or_else(|| refuse(SOURCE_ERROR))
        }
        Some(_) => Err(refuse(SOURCE_ERROR)),
    }
}

/// `category`: a caption or an index; absent is the current one.
fn parse_category(args: &Args) -> Result<Option<usize>, CmdError> {
    match args.get("category") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => find_category(s).map(Some).ok_or_else(|| refuse(CATEGORY_ERROR)),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|v| v as usize)
            .filter(|&v| v < CATEGORIES.len())
            .map(Some)
            .ok_or_else(|| refuse(CATEGORY_ERROR)),
        Some(_) => Err(refuse(CATEGORY_ERROR)),
    }
}

/// The project root and the game root the listing belongs to now.
fn scope(editor: &Editor) -> (String, String) {
    match Project::of(editor) {
        Ok((_, p)) => (p.root().to_owned(), p.game_root()),
        Err(_) => (String::new(), String::new()),
    }
}

fn location_json(loc: &Location) -> Value {
    json!({
        "source": loc.source.index(),
        "source_name": loc.source.wire(),
        "category": loc.category,
        "category_name": category_name(loc.category),
        "folder": loc.folder,
    })
}

/// Where the browser is seen and how that host shows its entries (`AppendBrowserHostJson`).
fn host_fields(out: &mut Map<String, Value>, host: &str, st: &Inner) {
    let target = if host == "panel" { Host::Panel } else { Host::Drawer };
    out.insert("visible".into(), json!(host != "none"));
    out.insert("host".into(), json!(host));
    out.insert("view".into(), json!(if st.host(target).list { "list" } else { "tiles" }));
    out.insert("tile_range".into(), json!([TILE_MIN, TILE_MAX]));
    out.insert("tile_unit".into(), json!("pt"));
}

/// Brings the browser into view (`MakeVisible`) and answers the host it is seen in.
async fn make_visible(editor: &Arc<Editor>) -> String {
    match actions::call(editor, "ui_drawer", &[("action", json!("open")), ("id", json!("content"))]).await {
        Ok(r) => r.get("host").and_then(Value::as_str).unwrap_or("none").to_owned(),
        Err(_) => "none".to_owned(),
    }
}

/// The catalogs a command may read, listed now. A source that cannot be listed is answered by
/// the navigation that needs it.
async fn prefetch(editor: &Arc<Editor>, browser: &ContentBrowser, keys: &[CatKey]) {
    for &k in keys {
        let _ = browser.cats.fresh(editor, k).await;
    }
}

fn nav_reply(st: &Inner, host: &str, now: u64) -> Value {
    let target = if host == "panel" { Host::Panel } else { Host::Drawer };
    let v = st.host(target);
    let mut out = Map::new();
    out.insert("open".into(), json!(st.host(Host::Panel).recent(now)));
    out.insert("tile".into(), json!(v.shown_tile(target).round()));
    out.insert("tile_fit".into(), json!(v.tile <= 0.0));
    host_fields(&mut out, host, st);
    out.insert("search".into(), json!(st.search));
    out.insert("sources_pane".into(), json!({ "on": st.sources_pane, "shown": st.sources_shown }));
    out.insert("can_add".into(), json!(st.add_blocked(!st.scope.0.is_empty()).is_none()));
    out.insert("current".into(), location_json(&st.loc));
    out.insert("back".into(), st.back.iter().map(location_json).collect());
    out.insert("forward".into(), st.forward.iter().map(location_json).collect());
    out.insert("favorites".into(), st.favorites.iter().map(location_json).collect());
    Value::Object(out)
}

const ACTIONS: &str = "action must be get, navigate, back, forward, up, home, refresh, favorite_add, favorite_remove, or favorite_open";

/// `content_browser_navigation`: history, Back / Forward / Up / Home, favorites, and the view
/// (tile size, list or tiles, the search, the Sources pane).
pub(crate) async fn content_browser_navigation(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.text("action").unwrap_or("get").to_ascii_lowercase();
    let has_folder = args.has("folder");
    let folder = args.str("folder").unwrap_or_default().to_owned();
    let source = parse_source(&args)?;
    let category = parse_category(&args)?;
    // the tile slider goes with any action; tile_fit=true fits the tiles to the host again
    let tile = args.f32("tile");
    let fitted = args.has("tile_fit");
    let fit = fitted && args.bool("tile_fit").unwrap_or(true);
    if tile.is_some() && fit {
        return Err(refuse("tile picks a size: tile_fit=true goes without it"));
    }
    if tile.is_some_and(|t| !(TILE_MIN..=TILE_MAX).contains(&t)) {
        return Err(refuse(TILE_RANGE_ERROR));
    }
    let view = args.str("view").map(str::to_ascii_lowercase);
    if view.as_deref().is_some_and(|v| v != "tiles" && v != "list") {
        return Err(refuse("view must be tiles or list"));
    }
    let search = args.str("search").map(str::to_owned);
    let pane = args.has("sources_pane").then(|| args.bool("sources_pane").unwrap_or(true));
    let index = args.i64("index").unwrap_or(-1);
    let known = [
        "get",
        "navigate",
        "back",
        "forward",
        "up",
        "home",
        "refresh",
        "favorite_add",
        "favorite_remove",
        "favorite_open",
    ];
    if !known.contains(&action.as_str()) {
        return Err(refuse(ACTIONS));
    }

    let browser = ContentBrowser::of(&ed);
    let scope_now = scope(&ed);
    // what the action may need listed
    let keys = {
        let mut st = browser.inner.lock();
        st.sync_scope(&scope_now.0, &scope_now.1, &browser.cats);
        let mut keys = vec![cat_key(&st.loc)];
        let cur = st.loc.clone();
        let named = Location::new(source.unwrap_or(cur.source), category.unwrap_or(cur.category), "");
        keys.push(cat_key(&named));
        keys.extend(st.back.last().map(cat_key));
        keys.extend(st.forward.last().map(cat_key));
        if let Some(f) = usize::try_from(index).ok().and_then(|i| st.favorites.get(i)) {
            keys.push(cat_key(f));
        }
        keys.sort_by_key(|k| (k.0.index(), k.1));
        keys.dedup();
        keys
    };
    prefetch(&ed, &browser, &keys).await;
    if action == "refresh" {
        let key = cat_key(&browser.inner.lock().loc);
        browser.cats.refresh(&ed, key);
        prefetch(&ed, &browser, &[key]).await;
    }

    let has_project = !scope_now.0.is_empty();
    let b = browser.clone();
    let act = action.clone();
    let (outcome, visible) = on_ui(&ed, move |shell, ctx| {
        let mut st = b.inner.lock();
        st.load_view(ctx);
        let now = ctx.cumulative_frame_nr();
        let host = st.target_host(now, shell.active() == gz_app::tabs::LEVEL);
        let mut viewed = false;
        if let Some(t) = tile {
            let v = st.host_mut(host);
            v.tile = t;
            v.list = false;
            viewed = true;
        } else if fitted {
            let v = st.host_mut(host);
            // a tile size either way, and a size asks for tiles
            v.tile = if fit { 0.0 } else { v.shown_tile(host) };
            v.list = false;
            viewed = true;
        }
        if let Some(v) = &view {
            st.host_mut(host).list = v == "list";
            viewed = true;
        }
        if let Some(s) = &search {
            st.search.clone_from(s);
        }
        if let Some(on) = pane {
            st.sources_pane = on;
            viewed = true;
        }
        if viewed {
            st.store_view(ctx);
        }
        let cats = b.cats.clone();
        let cur = st.loc.clone();
        let outcome = match act.as_str() {
            "navigate" => {
                let target =
                    Location::new(source.unwrap_or(cur.source), category.unwrap_or(cur.category), &folder);
                st.apply_location(target, true, &cats, has_project)
            }
            "back" => st.go_back(&cats, has_project),
            "forward" => st.go_forward(&cats, has_project),
            "up" => st.go_up(&cats, has_project),
            // the root of the source (and category) named, or of the one shown
            "home" if source.is_some() || category.is_some() => {
                let target =
                    Location::new(source.unwrap_or(cur.source), category.unwrap_or(cur.category), "");
                st.apply_location(target, true, &cats, has_project)
            }
            "home" => st.go_home(&cats, has_project),
            "refresh" => {
                st.pics.forget_all();
                Ok(())
            }
            "favorite_add" => {
                let mut target = cur.clone();
                if let Some(s) = source {
                    target.source = s;
                }
                if let Some(c) = category {
                    target.category = c;
                }
                if has_folder {
                    target.folder.clone_from(&folder);
                }
                st.add_favorite(target, &cats, has_project)
            }
            "favorite_remove" => match usize::try_from(index).ok().filter(|&i| i < st.favorites.len()) {
                Some(i) => {
                    st.favorites.remove(i);
                    Ok(())
                }
                None => Err("favorite index is out of range".to_owned()),
            },
            "favorite_open" => match usize::try_from(index).ok().and_then(|i| st.favorites.get(i).cloned()) {
                Some(f) => st.apply_location(f, true, &cats, has_project),
                None => Err("favorite index is out of range".to_owned()),
            },
            _ => Ok(()),
        };
        if !st.loc.same(&cur) {
            st.pics.cancel();
        }
        // moving through the folders and the view are visual: the browser comes up; get and the
        // favorite edits only read
        let moves = !matches!(act.as_str(), "get" | "favorite_add" | "favorite_remove");
        let visible = moves || viewed || search.is_some();
        ctx.request_repaint();
        Ok((outcome, visible))
    })
    .await?;
    outcome.map_err(refuse)?;
    let host = if visible { Some(make_visible(&ed).await) } else { None };
    let b = browser.clone();
    on_ui(&ed, move |_shell, ctx| {
        let st = b.inner.lock();
        let now = ctx.cumulative_frame_nr();
        let host = host.unwrap_or_else(|| st.visible_host(now).to_owned());
        Ok(nav_reply(&st, &host, now))
    })
    .await
}

/// Where an asset or a folder a caller named is: the source, the category, the path, a folder.
struct Found {
    source: Source,
    category: usize,
    path: String,
    folder: bool,
}

async fn find_named(
    ed: &Arc<Editor>,
    browser: &ContentBrowser,
    name: &str,
    source: Source,
    category: usize,
) -> Option<Found> {
    let folder_name = name.trim_end_matches('\\');
    let look = |cat: &crate::catalog::Catalog, by_reference: bool| -> Option<(String, bool)> {
        if let Some(f) = cat.file(name) {
            return Some((f.path.to_string(), false));
        }
        if !folder_name.is_empty()
            && let Some(n) = cat.folder(folder_name)
        {
            return Some((n.path.to_string(), true));
        }
        if by_reference && let Some(f) = cat.find_reference(name) {
            return Some((f.path.to_string(), false));
        }
        None
    };
    let key: CatKey = (source, if source == Source::Editor { category } else { 0 });
    if let Ok(cat) = browser.cats.fresh(ed, key).await
        && let Some((path, folder)) = look(&cat, source == Source::Editor)
    {
        return Some(Found { source, category, path, folder });
    }
    // the library is split into categories, and a caller naming an asset has no reason to know
    // which one holds it
    if source == Source::Editor {
        for c in (0..CATEGORIES.len()).filter(|&c| c != category && crate::service::category_has_files(c)) {
            if let Ok(cat) = browser.cats.fresh(ed, (Source::Editor, c)).await
                && let Some((path, false)) = look(&cat, true)
            {
                return Some(Found { source, category: c, path, folder: false });
            }
        }
    }
    None
}

/// `content_browser_open`: an asset or a folder revealed in the browser, optionally opened
/// (`RevealAsset`).
pub(crate) async fn content_browser_open(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Some(raw) = args.text("name") else {
        return Err(refuse("missing 'name' argument (see list_assets / darf_list)"));
    };
    let mut name = String::with_capacity(raw.len());
    for c in raw.chars() {
        let c = if c == '/' { '\\' } else { c };
        if !(c == '\\' && name.ends_with('\\')) {
            name.push(c);
        }
    }
    let src = parse_source(&args)?;
    let open = args.bool("open").unwrap_or(false);
    let browser = ContentBrowser::of(&ed);
    let scope_now = scope(&ed);
    let (source, category) = {
        let mut st = browser.inner.lock();
        st.sync_scope(&scope_now.0, &scope_now.1, &browser.cats);
        (src.unwrap_or(st.loc.source), st.loc.category)
    };
    let Some(found) = find_named(&ed, &browser, &name, source, category).await else {
        return Err(refuse("asset not found in this source"));
    };
    let has_project = !scope_now.0.is_empty();
    let b = browser.clone();
    let (fsrc, fcat, fpath, ffolder) = (found.source, found.category, found.path.clone(), found.folder);
    let landed = on_ui(&ed, move |_shell, _ctx| {
        let mut st = b.inner.lock();
        let origin = st.loc.clone();
        let folder = if ffolder { fpath.clone() } else { crate::model::parent_of(&fpath).to_owned() };
        st.apply_location(Location::new(fsrc, fcat, &folder), false, &b.cats, has_project).map_err(refuse)?;
        st.remember(&origin);
        st.search.clear();
        st.sel.only(Entry::new(&fpath, ffolder));
        // selecting it is only half a reveal: the next frame brings it into view
        st.scroll_to_selection = true;
        st.tree_open.insert(crate::service::tree_key(&folder));
        st.pics.cancel();
        Ok(st.loc.clone())
    })
    .await;
    let landed = landed?;
    if open {
        let root = browser.cats.peek(cat_key(&landed)).and_then(|c| c.disk_root.clone());
        let opens = !found.folder
            || actions::game_level(found.source, &found.path)
                .is_some_and(|l| found.path.len() == 7 + l.len());
        if opens {
            actions::open_for_mcp(&ed, found.source, found.category, &found.path, root)
                .await
                .map_err(refuse)?;
        }
    }
    let host = make_visible(&ed).await;
    let b = browser.clone();
    on_ui(&ed, move |_shell, _ctx| {
        let st = b.inner.lock();
        let mut out = Map::new();
        out.insert("source".into(), json!(st.loc.source.index()));
        out.insert("source_name".into(), json!(st.loc.source.wire()));
        out.insert("folder".into(), json!(st.loc.folder));
        out.insert("name".into(), json!(name));
        host_fields(&mut out, &host, &st);
        Ok(Value::Object(out))
    })
    .await
}

fn selection_reply(st: &Inner, browser: &ContentBrowser, now: u64) -> Value {
    let cat = browser.cats.peek(cat_key(&st.loc));
    let sel = st.sel.entries();
    let mut out = Map::new();
    out.insert("open".into(), json!(st.host(Host::Panel).recent(now)));
    out.insert("source".into(), json!(st.loc.source.index()));
    out.insert("source_name".into(), json!(st.loc.source.wire()));
    out.insert("folder".into(), json!(st.loc.folder));
    out.insert("count".into(), json!(sel.len()));
    out.insert("selection".into(), sel.iter().map(|e| json!(e.wire())).collect());
    let own = |e: &Entry| {
        if e.folder {
            None
        } else {
            cat.as_ref()
                .and_then(|c| c.file(&e.path))
                .and_then(|f| f.asset)
                .or_else(|| gz_project::kind_of(&e.path, None))
        }
    };
    // the drawn icon of every selected asset of the editor's own kinds
    let mut icons_ = Map::new();
    for e in sel {
        if let Some(kind) = own(e) {
            icons_.insert(e.wire(), json!(icons::own_name(kind)));
        }
    }
    out.insert("icons".into(), Value::Object(icons_));
    // the vector glyph of every selected entry, what its tile draws while it has no picture
    let mut glyphs = Map::new();
    for e in sel {
        glyphs.insert(
            e.wire(),
            json!(icons::tile_icon(&e.path, e.folder, own(e), st.loc.source, st.loc.category).name()),
        );
    }
    out.insert("glyphs".into(), Value::Object(glyphs));
    if let Some(r) = &st.rename {
        out.insert("rename".into(), json!({ "entry": r.entry.path, "text": r.text, "select": r.select }));
    }
    Value::Object(out)
}

/// `content_browser_selection`: what is selected; file-manager selection over the complete view;
/// the rename box; Show in Explorer.
pub(crate) async fn content_browser_selection(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str("action").unwrap_or_default().to_ascii_lowercase();
    let name = args.str("name").unwrap_or_default().to_owned();
    if action == "reveal" {
        let dry = args.bool("dry_run").unwrap_or(false);
        let root = ed.project_root();
        let path = ed.exec.io(move || actions::reveal_in_explorer(root, &name, dry)).await.map_err(refuse)?;
        return Ok(json!({ "path": path, "opened": !dry }));
    }
    let browser = ContentBrowser::of(&ed);
    let scope_now = scope(&ed);
    let modify = !action.is_empty() && action != "get";
    let mut view = None;
    if modify && action != "clear" {
        if !matches!(action.as_str(), "select" | "toggle" | "range" | "rename") {
            return Err(refuse("action must be select, toggle, range, rename or clear"));
        }
        if name.is_empty() {
            return Err(refuse("this action requires 'name'"));
        }
        let (loc, search) = {
            let mut st = browser.inner.lock();
            st.sync_scope(&scope_now.0, &scope_now.1, &browser.cats);
            (st.loc.clone(), st.search.clone())
        };
        view = Some(browser.fresh_view(&ed, &loc, &search).await.map_err(refuse)?);
    }
    let b = browser.clone();
    on_ui(&ed, move |_shell, ctx| {
        let mut st = b.inner.lock();
        if action == "clear" {
            st.sel.clear();
            st.scroll_to_selection = false;
            // an inline rename nobody finished goes with it, unrenamed
            st.rename = None;
        } else if let Some(view) = &view {
            let norm = normalize_name(&name);
            if norm.is_empty() {
                return Err(refuse("name resolves to an empty path"));
            }
            let mut hit: Option<Entry> = None;
            for e in view.entries.iter().filter(|e| e.entry.path.eq_ignore_ascii_case(&norm)) {
                if hit.is_some() {
                    return Err(refuse("entry is ambiguous in the current view"));
                }
                hit = Some(e.entry.clone());
            }
            let Some(entry) = hit else { return Err(refuse("entry is not in the complete current view")) };
            match action.as_str() {
                "rename" => {
                    if st.loc.source != Source::Project {
                        return Err(refuse("only the project's own content is renamed"));
                    }
                    st.sel.only(entry.clone());
                    actions::begin_rename(&mut st, &entry);
                    st.scroll_to_selection = true;
                }
                "range" => {
                    let order = view.order();
                    let Some(anchor) = st.sel.anchor().cloned() else {
                        return Err(refuse("range requires a selected anchor"));
                    };
                    if !order.contains(&anchor) {
                        return Err(refuse("selection anchor is not in the complete current view"));
                    }
                    st.sel.range(&order, &entry);
                    st.scroll_to_selection = true;
                }
                "toggle" => {
                    st.sel.toggle(entry.clone());
                    st.scroll_to_selection = st.sel.contains(&entry);
                }
                _ => {
                    st.sel.only(entry);
                    st.scroll_to_selection = true;
                }
            }
        }
        ctx.request_repaint();
        Ok(selection_reply(&st, &b, ctx.cumulative_frame_nr()))
    })
    .await
}

/// `content_browser_copy`: assets out of a read-only source into the project, never asking -
/// `overwrite` decides (`McpCopyToProject`).
pub(crate) async fn content_browser_copy(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let clean = |s: &str| normalize_name(s);
    let names = args.str("names").map(clean).unwrap_or_default();
    let folder = args.str("folder").map(clean).unwrap_or_default();
    let dst = args.str("dst").map(clean).unwrap_or_default();
    let src = parse_source(&args)?;
    let category = match args.text("category") {
        Some(c) => Some(find_category(c).ok_or_else(|| refuse(CATEGORY_ERROR))?),
        None => None,
    };
    let overwrite = args.bool("overwrite").unwrap_or(true);
    let browser = ContentBrowser::of(&ed);
    let scope_now = scope(&ed);
    let has_project = !scope_now.0.is_empty();
    let (cur, target) = {
        let mut st = browser.inner.lock();
        st.sync_scope(&scope_now.0, &scope_now.1, &browser.cats);
        let cur = st.loc.clone();
        let target = Location::new(src.unwrap_or(cur.source), category.unwrap_or(cur.category), "");
        (cur, target)
    };
    let failed = |files: u64, err: String| refuse(err).with_data(json!({ "files": files }));
    let differs =
        target.source != cur.source || (target.source == Source::Editor && target.category != cur.category);
    if differs {
        prefetch(&ed, &browser, &[cat_key(&target)]).await;
        let b = browser.clone();
        let t = target.clone();
        on_ui(&ed, move |_shell, _ctx| {
            let mut st = b.inner.lock();
            st.apply_location(t, true, &b.cats, has_project).map_err(refuse)?;
            st.pics.cancel();
            Ok(())
        })
        .await
        .map_err(|e| failed(0, e.message))?;
    }
    if target.source == Source::Project {
        return Err(failed(0, "the project's own Content is a destination, not a source".into()));
    }
    let key = cat_key(&target);
    let cat = browser.cats.fresh(&ed, key).await.map_err(|e| failed(0, e))?;
    // the folder goes on the clipboard as a folder (a copy into dst rebuilds the tree); without
    // dst its files are mirrored one by one
    let mut list: Vec<String> = Vec::new();
    if !folder.is_empty() {
        if cat.folder(&folder).is_none() {
            return Err(failed(0, "no such folder in this source".into()));
        }
        list.extend(cat.files_below(&folder).iter().map(|f| f.path.to_string()));
    } else {
        for n in names.split(';').map(|n| n.trim_matches(' ')).filter(|n| !n.is_empty()) {
            let resolved = if target.source == Source::Editor {
                cat.find_reference(n).map(|f| f.path.to_string())
            } else {
                cat.file(n).map(|f| f.path.to_string())
            };
            list.push(resolved.unwrap_or_else(|| n.to_owned()));
        }
    }
    if list.is_empty() {
        return Err(failed(0, "nothing to copy - pass 'names' or 'folder'".into()));
    }

    let (files, err) = if dst.is_empty() {
        mirror(&ed, target.source, target.category, &list, overwrite).await
    } else {
        if let Err((e, _)) = actions::call(&ed, "content_mkdir", &[("path", json!(dst))]).await {
            return Err(failed(0, e));
        }
        let entries: Vec<Entry> = if folder.is_empty() {
            list.iter().map(|p| Entry::new(p, false)).collect()
        } else {
            vec![Entry::new(&folder, true)]
        };
        let clips = actions::clips_of(&entries, &cat);
        let (files, err) = actions::transfer(&ed, target.source, false, clips, &dst).await;
        (files, err.unwrap_or_default())
    };
    if err.is_empty() { Ok(json!({ "files": files })) } else { Err(failed(files, err)) }
}

/// No destination: each asset where the engine expects it - a game file under the project's
/// `gamedata`, a library file with its companions into the folders the library keeps them in.
async fn mirror(
    ed: &Arc<Editor>,
    source: Source,
    category: usize,
    list: &[String],
    overwrite: bool,
) -> (u64, String) {
    let (mut files, mut failed, mut last) = (0u64, 0u32, String::new());
    for name in list {
        let mut one = String::new();
        if source == Source::Game {
            match actions::call(ed, "darf_copy", &[("path", json!(name)), ("overwrite", json!(overwrite))])
                .await
            {
                Ok(_) => files += 1,
                Err((e, r)) => {
                    one = if r.get("exists").and_then(Value::as_bool) == Some(true) && !overwrite {
                        "already exists (pass overwrite)".to_owned()
                    } else {
                        e
                    };
                }
            }
        } else if crate::model::category(category).lib.is_empty() {
            one = "this category has no file of its own - it lives inside a shared library file".to_owned();
        } else {
            let root = ed.paths.root.clone();
            let (n, cat) = (name.clone(), category);
            let backing = ed.exec.io(move || actions::library_files(&root, cat, &n)).await;
            if backing.is_empty() {
                one = "no file behind this reference".to_owned();
            }
            for (from, dst) in backing {
                let pairs = [
                    ("src", json!(from.display().to_string())),
                    ("dst", json!(dst)),
                    ("recursive", json!(false)),
                    ("overwrite", json!(overwrite)),
                ];
                match actions::call(ed, "content_copy", &pairs).await {
                    Ok(r) => files += r.get("files").and_then(Value::as_u64).unwrap_or(0),
                    Err((e, r)) => {
                        files += r.get("files").and_then(Value::as_u64).unwrap_or(0);
                        if one.is_empty() {
                            one = e;
                        }
                    }
                }
            }
        }
        if !one.is_empty() {
            failed += 1;
            last = one;
        }
    }
    let err = if failed > 0 { format!("{failed} item(s) failed, first: {last}") } else { String::new() };
    (files, err)
}
