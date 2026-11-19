//! `content.view`: what the Content Browser shows - the source, the Library's kind, the folder,
//! the search, the view, the selection, the rename box - to look at and to drive.
//!
//! A call awaits what it reads (the project's tree takes in what was written, a source is listed
//! when it was not), changes the view state on the UI thread at the start of the next frame
//! (`on_ui`), and brings the browser into view through `ui.drawer`, which raises the Level panel
//! when it shows the browser and opens the Content Drawer otherwise. It changes no file: an entry
//! answers the command that places it (`scene.place`) and the one that brings it into the project
//! (`file.import`).

use std::sync::Arc;

use gz_app::contract::{TILE_MAX, TILE_MIN};
use gz_app::on_ui;
use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, Value, json};
use gz_project::Project;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Map;

use crate::catalog::{Catalog, Origin as FileOrigin, Picture, View};
use crate::icons;
use crate::model::{self, HOME, Host, LIST_BELOW, Location, Source, find_kind, kind_word, normalize_name};
use crate::select::Entry;
use crate::service::{ContentBrowser, Inner, cat_key};

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// What `content.view` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ViewAction {
    /// Answers what the browser shows; the view settings given still apply.
    #[default]
    Get,
    /// Goes to `source` / `kind` / `folder`.
    Go,
    Back,
    Forward,
    Up,
    /// The project's root, or the Library's top.
    Home,
    /// Lists the source shown again.
    Refresh,
    /// Goes to the folder of `path` and selects it; `open=true` opens it as a double click does.
    Reveal,
    /// Selects `path` alone.
    Select,
    /// Adds `path` to the selection or takes it out.
    Toggle,
    /// Selects from the selection's anchor to `path`.
    Range,
    Clear,
    /// Opens the inline rename box on `path` (the project's entries only).
    Rename,
    /// Keeps the folder shown (or `source` / `kind` / `folder`) among the favorites.
    FavoriteAdd,
    /// Removes the favorite `index`.
    FavoriteRemove,
    /// Goes to the favorite `index`.
    FavoriteOpen,
}

/// Tiles or a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ViewKind {
    Tiles,
    List,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// What to do; `get` when left out.
    #[serde(default)]
    action: ViewAction,
    /// `project` or `library`; left out: the one shown.
    #[serde(default)]
    source: Option<String>,
    /// The Library's kind: objects, groups, textures, sounds, particles, spawn_sections,
    /// base_scenes, or home for the Library's top; left out: the one shown.
    #[serde(default)]
    kind: Option<String>,
    /// A folder of the source: project-relative, or below the Library's kind; `/` or `\`.
    #[serde(default)]
    folder: Option<String>,
    /// An entry of the folder shown (or, to reveal, of the source): its path in the source; a
    /// folder ends in `\`.
    #[serde(default)]
    path: Option<String>,
    /// reveal, in place of path: what a scene object names - an object's, a group's, a sound's
    /// or a particle effect's reference - with the Library's `kind`; the project's copy first,
    /// else the Library's entry.
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    /// reveal, in place of path: a spawn section, as `ref` (kind=spawn_sections).
    #[serde(default)]
    section: Option<String>,
    /// reveal: open the entry as a double click does.
    #[serde(default)]
    open: bool,
    /// The search of the folder shown; empty clears it.
    #[serde(default)]
    query: Option<String>,
    /// Tiles or a list, in the host the browser is seen in.
    #[serde(default)]
    view: Option<ViewKind>,
    /// The tile side, pt (48..192; under 64 the folder is drawn as a list).
    #[serde(default)]
    tile: Option<f32>,
    /// true fits the tiles to the host again; false keeps the size shown as the one picked.
    #[serde(default)]
    tile_fit: Option<bool>,
    /// Shows or hides the Sources pane.
    #[serde(default)]
    sources_pane: Option<bool>,
    /// favorite_remove, favorite_open: the favorite, from 0.
    #[serde(default)]
    index: Option<usize>,
    /// At most this many entries of the folder shown (100 when left out).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many entries first.
    #[serde(default)]
    offset: Option<usize>,
}

/// Registers `content.view`.
pub(crate) fn register(reg: &mut Registry) {
    reg.area("content", "The Content Browser: the project's files and the Library, as the window shows them");
    reg.cmd(
        Spec::view(
            "content.view",
            "Shows or drives what the Content Browser shows: source, Library kind, folder, search, view, selection.",
        )
        .example("action=go source=library kind=spawn_sections folder=weapons"),
        content_view,
    );
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
        "source": loc.source.wire(),
        "kind": if loc.source == Source::Library { kind_word(loc.category) } else { "" },
        "folder": loc.folder,
    })
}

/// Brings the browser into view and answers the host it is seen in.
async fn make_visible(editor: &Arc<Editor>) -> String {
    let mut args = Map::new();
    args.insert("action".into(), json!("open"));
    args.insert("id".into(), json!("content"));
    let r = editor.call("ui.drawer", args, editor.ui_origin()).await;
    r.get("host").and_then(Value::as_str).unwrap_or("none").to_owned()
}

/// The source and kind a call names, the ones shown filling in.
fn named(p: &ViewParams, cur: &Location) -> Result<(Source, usize), CmdError> {
    let source = match p.source.as_deref() {
        None => cur.source,
        Some(s) => Source::parse_name(s).ok_or_else(|| refuse("source must be project or library"))?,
    };
    let kind = match p.kind.as_deref() {
        None if source == cur.source => cur.category,
        None => HOME,
        Some(k) => find_kind(k).ok_or_else(|| {
            refuse("kind must be objects, groups, textures, sounds, particles, spawn_sections, base_scenes or home")
        })?,
    };
    Ok((source, if source == Source::Library { kind } else { 0 }))
}

/// The entry a reference names: the project's own copy first, then the Library's.
async fn by_reference(
    ed: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    p: &ViewParams,
    has_project: bool,
) -> Result<(Source, usize, Entry), CmdError> {
    let want = p.reference.as_deref().or(p.section.as_deref()).unwrap_or_default().trim().replace('/', "\\");
    let category = p.kind.as_deref().and_then(find_kind).ok_or_else(|| {
        refuse(
            "a reveal by ref names the Library's kind: objects, groups, sounds, particles or spawn_sections",
        )
    })?;
    let placed = model::kind(category).filter(|k| k.placed).map(|k| k.payload).ok_or_else(|| {
        refuse("a reveal by ref takes kind objects, groups, sounds, particles or spawn_sections")
    })?;
    // a reference as a scene names it: no folder of the kind, no extension (a project's group is
    // its file `rawdata\groups\x.group`, a scene's group `x`; a sound `x.ogg` is `x`)
    let short = |v: &str| {
        let v = v.trim().replace('/', "\\");
        let below = match v.get(..15) {
            Some(head) if head.eq_ignore_ascii_case("rawdata\\groups\\") => v[15..].to_owned(),
            _ => v,
        };
        crate::catalog::stem(&below).to_ascii_lowercase()
    };
    let wanted = short(&want);
    let find = |cat: &Catalog| {
        cat.files()
            .find(|f| {
                crate::actions::placeable(cat, &f.path)
                    .is_some_and(|(kind, _, value)| kind == placed && short(&value) == wanted)
            })
            .map(|f| Entry::new(&f.path, false))
    };
    let mut places = Vec::with_capacity(2);
    if has_project && matches!(category, model::OBJECTS | model::GROUPS) {
        places.push((Source::Project, 0));
    }
    places.push((Source::Library, category));
    for (source, cat_kind) in places {
        let at = Location::new(source, cat_kind, "");
        let cat = browser.cats.fresh(ed, cat_key(&at)).await.map_err(refuse)?;
        if let Some(e) = find(&cat) {
            return Ok((source, cat_kind, e));
        }
    }
    Err(CmdError::not_found(format!(
        "{want} is neither in the project nor in the Library's {}.",
        model::kind(category).map_or("entries", |k| k.caption)
    )))
}

/// An entry of the source: what `path` names, as the catalog spells it.
fn find_entry(cat: &Catalog, raw: &str) -> Option<Entry> {
    let name = normalize_name(raw);
    let folder = raw.ends_with(['\\', '/']);
    if !folder && let Some(f) = cat.file(&name).or_else(|| cat.find_reference(&name)) {
        return Some(Entry::new(&f.path, false));
    }
    cat.folder(&name).filter(|n| !n.path.is_empty()).map(|n| Entry::new(&n.path, true))
}

/// One entry as the answer lists it: what the tile shows and what a caller does with it.
fn entry_json(cat: Option<&Catalog>, view: &crate::catalog::ViewEntry) -> Value {
    let e = &view.entry;
    let mut out = Map::new();
    out.insert("path".into(), json!(e.wire()));
    out.insert("name".into(), json!(view.label.as_deref().unwrap_or(model::leaf_of(&e.path))));
    out.insert("folder".into(), json!(e.folder));
    out.insert("glyph".into(), json!(view.glyph.name()));
    if let Some(kind) = view.own {
        out.insert("icon".into(), json!(icons::own_name(kind)));
    }
    out.insert(
        "picture".into(),
        json!(match view.picture {
            Picture::None => "none",
            Picture::Image => "image",
            Picture::Model => "model",
            Picture::Thumb => "thumbnail",
        }),
    );
    if view.wanted {
        out.insert("wanted".into(), json!(true));
    }
    if view.ship.veiled() {
        out.insert("excluded".into(), json!(true));
    }
    if let Some(cat) = cat {
        let asset = crate::payload_of(cat, &e.path, e.folder);
        out.insert("payload".into(), json!({ "kind": asset.kind, "path": asset.path }));
        if !e.folder {
            if let Some((kind, field, value)) = crate::actions::placeable(cat, &e.path) {
                out.insert("place".into(), json!({ "kind": kind, field: value }));
            }
            if cat.source == Source::Library
                && let Some(f) = cat.file(&e.path)
            {
                // a level of the game: Open is the project's scene over it
                if let Some(level) = crate::library::game_level(cat, &e.path) {
                    out.insert("level".into(), json!(level));
                }
                let import = match (cat.category, f.origin) {
                    (model::SCENES, _) => None,
                    (_, FileOrigin::Game) => cat.game_path(&e.path).map(|p| json!({ "from": "game", "path": p.replace('\\', "/") })),
                    (_, FileOrigin::Disk) => model::kind(cat.category).map(|k| {
                        json!({ "from": "library", "path": format!("{}\\{}", k.library_folder, e.path).replace('\\', "/") })
                    }),
                    (_, FileOrigin::None) => None,
                };
                if let Some(i) = import {
                    out.insert("import".into(), i);
                }
                if let Some(c) = &view.note {
                    out.insert("note".into(), json!(c));
                }
            }
        }
    }
    Value::Object(out)
}

/// The answer: where the browser is, how its host shows it, the folder's entries, the selection.
fn reply(
    st: &Inner,
    browser: &ContentBrowser,
    host: &str,
    now: u64,
    view: Option<&View>,
    p: &ViewParams,
) -> Value {
    let target = if host == "panel" { Host::Panel } else { Host::Drawer };
    let v = st.host(target);
    let cat = browser.cats.peek(cat_key(&st.loc));
    let mut out = Map::new();
    out.insert("host".into(), json!(host));
    out.insert("visible".into(), json!(host != "none"));
    out.insert("open".into(), json!(st.host(Host::Panel).recent(now)));
    out.insert("current".into(), location_json(&st.loc));
    out.insert("source".into(), json!(st.loc.source.wire()));
    out.insert(
        "kind".into(),
        json!(if st.loc.source == Source::Library { kind_word(st.loc.category) } else { "" }),
    );
    out.insert("folder".into(), json!(st.loc.folder));
    out.insert("query".into(), json!(st.search));
    out.insert("view".into(), json!(if v.shows_list(target) { "list" } else { "tiles" }));
    out.insert("tile".into(), json!(v.shown_tile(target).round()));
    out.insert("tile_fit".into(), json!(v.tile <= 0.0));
    out.insert("tile_range".into(), json!([TILE_MIN, TILE_MAX]));
    out.insert("list_below".into(), json!(LIST_BELOW));
    out.insert("sources_pane".into(), json!({ "on": st.sources_pane, "shown": st.sources_shown }));
    out.insert("can_add".into(), json!(st.add_blocked(!st.scope.0.is_empty()).is_none()));
    out.insert("back".into(), st.back.iter().map(location_json).collect());
    out.insert("forward".into(), st.forward.iter().map(location_json).collect());
    out.insert("favorites".into(), st.favorites.iter().map(location_json).collect());
    if let Some(note) = cat.as_ref().and_then(|c| c.note.clone()).filter(|_| !st.loc.home()) {
        out.insert("note".into(), json!(note));
    }
    if let Some(view) = view {
        let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(100).max(1));
        let shown: Vec<Value> = view
            .entries
            .iter()
            .skip(offset)
            .take(limit)
            .map(|e| entry_json(if st.loc.home() { None } else { cat.as_deref() }, e))
            .collect();
        out.insert("entries".into(), Value::Array(shown));
        out.insert("total".into(), json!(view.entries.len()));
        out.insert("folders".into(), json!(view.folders));
        out.insert("files".into(), json!(view.files));
    }
    let sel = st.sel.entries();
    out.insert("selection".into(), sel.iter().map(|e| json!(e.wire())).collect());
    let own = |e: &Entry| {
        if e.folder { None } else { cat.as_ref().and_then(|c| c.file(&e.path)).and_then(|f| f.asset) }
    };
    let mut icons_ = Map::new();
    let mut glyphs = Map::new();
    let kind_icon = st.loc.lib_kind().map(|k| k.icon);
    for e in sel {
        if let Some(kind) = own(e) {
            icons_.insert(e.wire(), json!(icons::own_name(kind)));
        }
        glyphs.insert(e.wire(), json!(icons::tile_icon(&e.path, e.folder, own(e), kind_icon).name()));
    }
    out.insert("icons".into(), Value::Object(icons_));
    out.insert("glyphs".into(), Value::Object(glyphs));
    if let Some(r) = &st.rename {
        out.insert("rename".into(), json!({ "entry": r.entry.path, "text": r.text, "select": r.select }));
    }
    Value::Object(out)
}

async fn content_view(ed: Arc<Editor>, p: ViewParams, _origin: Origin) -> Reply {
    if p.tile.is_some_and(|t| !(TILE_MIN..=TILE_MAX).contains(&t)) {
        return Err(CmdError::invalid("tile takes 48..192 (pt)"));
    }
    if p.tile.is_some() && p.tile_fit == Some(true) {
        return Err(CmdError::invalid("tile picks a size: tile_fit=true goes without it"));
    }
    let browser = ContentBrowser::of(&ed);
    let scope_now = scope(&ed);
    let has_project = !scope_now.0.is_empty();
    let cur = {
        let mut st = browser.inner.lock();
        st.sync_scope(&scope_now.0, &scope_now.1, &browser.cats);
        st.loc.clone()
    };
    let (source, kind) = named(&p, &cur)?;
    let act = p.action;
    // what the action needs listed first
    let mut target: Option<Location> = None;
    let mut entry: Option<Entry> = None;
    let mut order: Option<Vec<Entry>> = None;
    match act {
        ViewAction::Go => {
            let folder = p.folder.as_deref().map(normalize_name).unwrap_or_default();
            let t = Location::new(source, kind, &folder);
            if !t.home() && !folder.is_empty() {
                browser.cats.fresh(&ed, cat_key(&t)).await.map_err(refuse)?;
            }
            target = Some(t);
        }
        ViewAction::Reveal if p.reference.is_some() || p.section.is_some() => {
            let (src, cat_kind, e) = by_reference(&ed, &browser, &p, has_project).await?;
            let folder = model::parent_of(&e.path).to_owned();
            target = Some(Location::new(src, cat_kind, &folder));
            entry = Some(e);
        }
        ViewAction::Reveal => {
            let raw =
                p.path.as_deref().ok_or_else(|| CmdError::invalid("reveal takes path, ref or section"))?;
            let at = Location::new(source, kind, "");
            if at.home() {
                return Err(refuse("name the Library's kind to reveal an entry of it (kind=)"));
            }
            let cat = browser.cats.fresh(&ed, cat_key(&at)).await.map_err(refuse)?;
            let e = find_entry(&cat, raw).ok_or_else(|| {
                CmdError::not_found(match source {
                    Source::Project => format!("{raw} is not among the project's files."),
                    Source::Library => format!("{raw} is not in the Library."),
                })
            })?;
            let folder = if e.folder { e.path.to_string() } else { model::parent_of(&e.path).to_owned() };
            target = Some(Location::new(source, kind, &folder));
            entry = Some(e);
        }
        ViewAction::Select | ViewAction::Toggle | ViewAction::Range | ViewAction::Rename => {
            let raw = p.path.as_deref().ok_or_else(|| CmdError::invalid("this action takes path"))?;
            let search = browser.inner.lock().search.clone();
            let view = browser.fresh_view(&ed, &cur, &search).await.map_err(refuse)?;
            let norm = normalize_name(raw);
            let folder = raw.ends_with(['\\', '/']);
            let hit = view
                .entries
                .iter()
                .find(|e| e.entry.path.eq_ignore_ascii_case(&norm) && (e.entry.folder || !folder))
                .map(|e| e.entry.clone());
            entry =
                Some(hit.ok_or_else(|| CmdError::not_found(format!("{raw} is not in the folder shown")))?);
            order = Some(view.order());
        }
        ViewAction::Refresh => {
            browser.cats.refresh(&ed, cat_key(&cur));
        }
        ViewAction::FavoriteAdd => {
            let folder = p.folder.as_deref().map(normalize_name).unwrap_or_else(|| cur.folder.clone());
            let t = Location::new(source, kind, &folder);
            if !t.home() {
                browser.cats.fresh(&ed, cat_key(&t)).await.map_err(refuse)?;
            }
            target = Some(t);
        }
        _ => {}
    }
    // the history's next stop may need its listing
    let next_stops = {
        let st = browser.inner.lock();
        [st.back.last().cloned(), st.forward.last().cloned()]
    };
    for loc in next_stops
        .into_iter()
        .flatten()
        .filter(|l| !l.home() && matches!(act, ViewAction::Back | ViewAction::Forward))
    {
        let _ = browser.cats.fresh(&ed, cat_key(&loc)).await;
    }
    if matches!(act, ViewAction::Up) && !cur.home() && !cur.folder.is_empty() {
        let _ = browser.cats.fresh(&ed, cat_key(&cur)).await;
    }

    let opened = entry.clone();
    let b = browser.clone();
    let query = p.query.clone();
    let (view_kind, tile, tile_fit, pane, index) = (p.view, p.tile, p.tile_fit, p.sources_pane, p.index);
    let moved = on_ui(&ed, move |shell, ctx| {
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
        } else if let Some(fit) = tile_fit {
            let v = st.host_mut(host);
            // a tile size either way, and a size asks for tiles
            v.tile = if fit { 0.0 } else { v.shown_tile(host).max(LIST_BELOW) };
            v.list = false;
            viewed = true;
        }
        if let Some(k) = view_kind {
            st.host_mut(host).list = k == ViewKind::List;
            viewed = true;
        }
        if let Some(on) = pane {
            st.sources_pane = on;
            viewed = true;
        }
        if viewed {
            st.store_view(ctx);
        }
        if let Some(q) = &query {
            st.search.clone_from(q);
        }
        let cats = b.cats.clone();
        let before = st.loc.clone();
        let outcome: Result<(), String> = match act {
            ViewAction::Go => st.apply_location(target.clone().unwrap_or_default(), true, &cats, has_project),
            ViewAction::Back => st.go_back(&cats, has_project),
            ViewAction::Forward => st.go_forward(&cats, has_project),
            ViewAction::Up => st.go_up(&cats, has_project),
            ViewAction::Home => st.go_home(&cats, has_project),
            ViewAction::Refresh => {
                st.pics.forget_all();
                Ok(())
            }
            ViewAction::Reveal => {
                let origin = st.loc.clone();
                st.apply_location(target.clone().unwrap_or_default(), false, &cats, has_project).map(|()| {
                    st.remember(&origin);
                    st.search.clear();
                    if let Some(e) = &entry {
                        st.sel.only(e.clone());
                        let folder = st.loc.folder.clone();
                        st.tree_open.insert(crate::service::tree_key(&folder));
                    }
                    // selecting it is only half a reveal: the next frame brings it into view
                    st.scroll_to_selection = true;
                })
            }
            ViewAction::Select => {
                if let Some(e) = entry.clone() {
                    st.sel.only(e);
                    st.scroll_to_selection = true;
                }
                Ok(())
            }
            ViewAction::Toggle => {
                if let Some(e) = entry.clone() {
                    st.sel.toggle(e.clone());
                    st.scroll_to_selection = st.sel.contains(&e);
                }
                Ok(())
            }
            ViewAction::Range => match (entry.clone(), st.sel.anchor().cloned(), order.as_ref()) {
                (Some(e), Some(anchor), Some(order)) if order.contains(&anchor) => {
                    st.sel.range(order, &e);
                    st.scroll_to_selection = true;
                    Ok(())
                }
                (_, Some(_), _) => Err("the selection's anchor is not in the folder shown".to_owned()),
                _ => Err("range needs a selected entry to start from".to_owned()),
            },
            ViewAction::Clear => {
                st.sel.clear();
                st.scroll_to_selection = false;
                // an inline rename nobody finished goes with it, unrenamed
                st.rename = None;
                Ok(())
            }
            ViewAction::Rename => {
                if st.loc.source != Source::Project {
                    Err("only the project's own content is renamed".to_owned())
                } else {
                    if let Some(e) = entry.clone() {
                        st.sel.only(e.clone());
                        crate::actions::begin_rename(&mut st, &e);
                        st.scroll_to_selection = true;
                    }
                    Ok(())
                }
            }
            ViewAction::FavoriteAdd => {
                st.add_favorite(target.clone().unwrap_or_default(), &cats, has_project)
            }
            ViewAction::FavoriteRemove => match index.filter(|&i| i < st.favorites.len()) {
                Some(i) => {
                    st.favorites.remove(i);
                    Ok(())
                }
                None => Err("no favorite has that index".to_owned()),
            },
            ViewAction::FavoriteOpen => match index.and_then(|i| st.favorites.get(i).cloned()) {
                Some(f) => st.apply_location(f, true, &cats, has_project),
                None => Err("no favorite has that index".to_owned()),
            },
            ViewAction::Get => Ok(()),
        };
        if !st.loc.same(&before) {
            st.pics.cancel();
        }
        ctx.request_repaint();
        // what moves or changes the view brings the browser up; get and the favorites' edits read
        let moves = !matches!(act, ViewAction::Get | ViewAction::FavoriteAdd | ViewAction::FavoriteRemove)
            || viewed
            || query.is_some();
        outcome.map(|()| moves).map_err(refuse)
    })
    .await?;
    let shown_key = cat_key(&browser.inner.lock().loc);
    if p.open
        && let Some(e) = &opened
        && !e.folder
        && let Some(cat) = browser.cats.peek(shown_key)
    {
        crate::actions::open_for_mcp(&ed, &cat, &e.path).await.map_err(refuse)?;
    }
    let host = if moved { Some(make_visible(&ed).await) } else { None };
    // the answer lists the folder shown as it is now
    let (loc, search) = {
        let st = browser.inner.lock();
        (st.loc.clone(), st.search.clone())
    };
    let view = if loc.source == Source::Project && !has_project {
        None
    } else {
        browser.fresh_view(&ed, &loc, &search).await.ok()
    };
    let b = browser.clone();
    on_ui(&ed, move |_shell, ctx| {
        let st = b.inner.lock();
        let now = ctx.cumulative_frame_nr();
        let host = host.unwrap_or_else(|| st.visible_host(now).to_owned());
        Ok(reply(&st, &b, &host, now, view.as_deref(), &p))
    })
    .await
}
