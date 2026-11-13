//! What the browser does: every action is an editor command (`content_rename`, `content_move`,
//! `content_copy`, `content_delete`, `content_mkdir`, `content_create`, `mod_exclude`,
//! `darf_copy`, `ui_tab`, `open_scene`, `place_object`, `game_link`), so the window and MCP share
//! one code path. The browser changes no file itself.

use std::path::PathBuf;
use std::sync::Arc;

use gz_app::{DocCx, Drawer, OpenRequest, PanelOp};
use gz_editor::{Args, Editor, Value, json};
use gz_ui::dialog::{self, Answer, Asker, ConfirmSpec};

use crate::catalog::Catalog;
use crate::model::{LEVELS, Location, OBJECTS, Source, VISUALS, category, leaf_of, parent_of};
use crate::select::Entry;
use crate::service::{CatKey, Clip, Clipboard, ContentBrowser, Inner, Rename};
use crate::{PANEL_ID, icons};

/// Who answers the browser's questions from the window.
#[must_use]
pub(crate) fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

fn args(pairs: &[(&str, Value)]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert((*k).to_owned(), v.clone());
    }
    a
}

/// Runs a command and splits its answer: the fields, or the error and the fields.
pub(crate) async fn call(
    editor: &Arc<Editor>,
    name: &str,
    pairs: &[(&str, Value)],
) -> Result<Value, (String, Value)> {
    let reply = editor.call(name, args(pairs), editor.ui_origin()).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        let why = reply.get("error").and_then(Value::as_str).unwrap_or("the command failed").to_owned();
        Err((why, reply))
    }
}

fn first_reason(reply: &Value) -> String {
    reply
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|e| e.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn count(reply: &Value, key: &str) -> u64 {
    reply.get(key).and_then(Value::as_u64).unwrap_or(0)
}

// ---- opening ----------------------------------------------------------------------------------

/// What a double click opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Open {
    /// A document tab of the shell.
    Doc(OpenRequest),
    /// A scene, through the scene's command.
    Scene(String),
}

/// `meshes\dynamics\box\box_1a.ogf` -> `dynamics\box\box_1a`: the visual name a model is opened by
/// (`ToVisualName`).
#[must_use]
pub(crate) fn visual_name(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let rest = if lower.starts_with("meshes\\") || lower.starts_with("meshes/") { &path[7..] } else { path };
    let rest = icons::extension(rest).map_or(rest, |e| &rest[..rest.len() - e.len()]);
    rest.replace('/', "\\")
}

/// A game entry under `levels\` belongs to one of the game's levels; the folder right under it is
/// the level.
#[must_use]
pub(crate) fn game_level(source: Source, path: &str) -> Option<&str> {
    if source != Source::Game {
        return None;
    }
    let rest = path.get(..7).filter(|h| h.eq_ignore_ascii_case("levels\\")).and(path.get(7..))?;
    let name = rest.split('\\').next().filter(|n| !n.is_empty())?;
    Some(name)
}

/// What opening an entry means (`OpenAsset`). `root` is where the source keeps its files on disk.
pub(crate) fn what_opens(
    source: Source,
    cat: usize,
    path: &str,
    root: Option<&std::path::Path>,
) -> Result<Open, String> {
    if path.is_empty() {
        return Err("empty asset name".into());
    }
    let ext = icons::extension(path).map(str::to_ascii_lowercase).unwrap_or_default();
    // a level IS the scene: opening one loads it
    let editor_level = source == Source::Editor && cat == LEVELS;
    if ext == ".level" || editor_level {
        let scene = match (source, root) {
            (Source::Project, Some(r)) => r.join(path).display().to_string(),
            (Source::Editor, Some(r)) => {
                let file = if ext == ".level" { path.to_owned() } else { format!("{path}.level") };
                r.join(file).display().to_string()
            }
            _ => path.to_owned(),
        };
        return Ok(Open::Scene(scene));
    }
    if let Some(level) = game_level(source, path) {
        // the game's levels are compiled: the editor opens the source scene of that name
        return Ok(Open::Scene(level.to_owned()));
    }
    let model =
        ext == ".ogf" || ext == ".object" || (source == Source::Editor && (cat == VISUALS || cat == OBJECTS));
    if model {
        let target = match source {
            Source::Game => visual_name(path),
            Source::Editor if cat == VISUALS => visual_name(path),
            _ => path.to_owned(),
        };
        return Ok(Open::Doc(OpenRequest {
            arg: Some(source.wire().to_owned()),
            ..OpenRequest::file("model", &target)
        }));
    }
    let project_only = |what: &str| -> Result<(), String> {
        if source == Source::Project {
            Ok(())
        } else {
            Err(format!("{path}: {what} open from the project only"))
        }
    };
    match ext.as_str() {
        ".pdaasset" => {
            project_only("PDA layouts")?;
            return Ok(Open::Doc(OpenRequest::file("pda", path)));
        }
        ".npcasset" => {
            project_only("characters")?;
            return Ok(Open::Doc(OpenRequest::file("characters", path)));
        }
        ".nqasset" | ".behasset" => {
            project_only("quest graphs")?;
            return Ok(Open::Doc(OpenRequest::file(if ext == ".nqasset" { "quest" } else { "beh" }, path)));
        }
        _ => {}
    }
    let texture = icons::is_image(path) || (source == Source::Editor && (cat == 4 || cat == 5));
    if texture {
        return Ok(Open::Doc(OpenRequest {
            arg: Some(source.wire().to_owned()),
            ..OpenRequest::file("texture", path)
        }));
    }
    Err(format!("no viewer for this asset kind yet: '{path}'"))
}

/// A double click from the window: into a folder, a game level's scene, an asset's tab.
pub(crate) fn open_entry(
    cx: &mut DocCx<'_>,
    st: &mut Inner,
    browser: &ContentBrowser,
    entry: &Entry,
    has_project: bool,
) {
    let source = st.loc.source;
    let cat = st.loc.category;
    if entry.folder {
        // a game level is a folder on disk and a scene to the author
        if game_level(source, &entry.path).is_some_and(|l| entry.path.len() == 7 + l.len()) {
            open_asset(cx, source, cat, &entry.path, None);
            return;
        }
        // into the folder a search found: the search has done its job
        st.search.clear();
        let target = Location::new(source, cat, &entry.path);
        if st.apply_location(target, true, &browser.cats, has_project).is_ok() {
            st.pics.cancel();
        }
        return;
    }
    st.sel.only(entry.clone());
    let root = browser.cats.peek(crate::service::cat_key(&st.loc)).and_then(|c| c.disk_root.clone());
    open_asset(cx, source, cat, &entry.path, root.as_deref());
}

fn open_asset(cx: &mut DocCx<'_>, source: Source, cat: usize, path: &str, root: Option<&std::path::Path>) {
    match what_opens(source, cat, path, root) {
        Ok(Open::Doc(req)) => cx.requests.open(req),
        Ok(Open::Scene(scene)) => cx.editor.post("open_scene", args(&[("scene", json!(scene))])),
        Err(e) if e.starts_with("no viewer") => {
            cx.editor.notices.note(format!("No viewer for this kind of file yet: '{path}'"))
        }
        Err(e) => cx.editor.notices.error(e),
    }
}

/// The same for MCP: the outcome comes back instead of a notice.
pub(crate) async fn open_for_mcp(
    editor: &Arc<Editor>,
    source: Source,
    cat: usize,
    path: &str,
    root: Option<PathBuf>,
) -> Result<(), String> {
    match what_opens(source, cat, path, root.as_deref())? {
        Open::Doc(req) => {
            let mut pairs = vec![("action", json!("open")), ("kind", json!(req.kind))];
            if let Some(p) = &req.path {
                pairs.push(("path", json!(p)));
            }
            if let Some(s) = &req.arg {
                pairs.push(("source", json!(s)));
            }
            call(editor, "ui_tab", &pairs).await.map(|_| ()).map_err(|(e, _)| format!("{path}: {e}"))
        }
        Open::Scene(scene) => {
            call(editor, "open_scene", &[("scene", json!(scene))]).await.map(|_| ()).map_err(|(e, _)| e)
        }
    }
}

/// The library reference a placeable entry is placed by (`ResolveDropRef`): a project
/// `rawdata\objects` file or an Editor Objects entry, the extension clamped.
#[must_use]
pub(crate) fn placeable_ref(source: Source, cat: usize, path: &str) -> Option<String> {
    let is_object = icons::extension(path).is_some_and(|e| e.eq_ignore_ascii_case(".object"));
    let strip = |p: &str| icons::extension(p).map_or(p, |e| &p[..p.len() - e.len()]).to_owned();
    match source {
        Source::Project if is_object => path
            .get(..16)
            .filter(|h| h.eq_ignore_ascii_case("rawdata\\objects\\"))
            .and(path.get(16..))
            .map(strip),
        Source::Editor if cat == OBJECTS && is_object => Some(strip(path)),
        _ => None,
    }
}

// ---- the inline rename --------------------------------------------------------------------------

/// Opens the rename box on an entry: a folder is its whole name; one of the editor's own assets
/// shows its name without the extension; any other file keeps its extension out of the selection
/// (`BeginRename`).
pub(crate) fn begin_rename(st: &mut Inner, entry: &Entry) {
    if st.loc.source != Source::Project {
        return;
    }
    let leaf = leaf_of(&entry.path).to_owned();
    let (text, select) = if entry.folder {
        let n = leaf.len();
        (leaf, n)
    } else if let Some(own) = gz_project::own_extension(&leaf) {
        let name = leaf[..leaf.len() - own.len()].to_owned();
        let n = name.len();
        (name, n)
    } else {
        let n = leaf.rfind('.').filter(|&d| d > 0).unwrap_or(leaf.len());
        (leaf, n)
    };
    st.rename = Some(Rename { entry: entry.clone(), text, select, focus: true });
}

/// The box is done: `content_rename` with what it holds (`CommitRename`). The browser follows a
/// renamed folder it shows or holds in its history.
pub(crate) fn commit_rename(editor: &Arc<Editor>, browser: &Arc<ContentBrowser>, st: &mut Inner) {
    let Some(r) = st.rename.take() else { return };
    let leaf = leaf_of(&r.entry.path);
    let shown = if r.entry.folder {
        leaf.to_owned()
    } else {
        gz_project::own_extension(leaf).map_or(leaf.to_owned(), |e| leaf[..leaf.len() - e.len()].to_owned())
    };
    if r.text == shown {
        return;
    }
    let was = r.entry.path.to_string();
    let inbox = browser.inbox.clone();
    let ed = editor.clone();
    editor.exec.spawn(async move {
        match call(&ed, "content_rename", &[("path", json!(was)), ("name", json!(r.text))]).await {
            Ok(reply) => {
                let to = reply.get("path").and_then(Value::as_str).unwrap_or("").replace('/', "\\");
                let parent = parent_of(&was);
                let now = if parent.is_empty() {
                    leaf_of(&to).to_owned()
                } else {
                    format!("{parent}\\{}", leaf_of(&to))
                };
                if !now.is_empty() && now != was {
                    inbox.lock().renamed = Some((was, now));
                }
            }
            Err((e, _)) => ed.notices.error(format!("Cannot rename '{was}': {e}.")),
        }
        ed.exec.wake().ping();
    });
}

// ---- delete ------------------------------------------------------------------------------------

fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The confirmation of a deletion: it names what goes (`DeleteSelection`).
#[must_use]
pub(crate) fn delete_spec(entries: &[Entry], cat: Option<&Catalog>) -> ConfirmSpec {
    let mut folders = 0usize;
    let mut files = 0usize;
    let mut inside = 0usize;
    let mut items = Vec::with_capacity(entries.len());
    for e in entries {
        items.push(e.wire());
        if e.folder {
            folders += 1;
            inside += cat.map_or(0, |c| c.files_below(&e.path).len());
        } else {
            files += 1;
        }
    }
    let headline = match entries {
        [one] if one.folder => format!("Delete the folder {} and everything in it?", leaf_of(&one.path)),
        [one] => format!("Delete {}?", leaf_of(&one.path)),
        _ if folders > 0 && files > 0 => format!(
            "Delete {} and {}?",
            counted(folders, "folder", "folders"),
            counted(files, "file", "files")
        ),
        _ if folders > 0 => format!("Delete {}?", counted(folders, "folder", "folders")),
        _ => format!("Delete {}?", counted(files, "file", "files")),
    };
    let consequence = if inside > 0 {
        format!(
            "{}{}. Deleted for good - this cannot be undone.",
            if folders == 1 { "It holds " } else { "The folders hold " },
            counted(inside, "file", "files")
        )
    } else {
        String::new()
    };
    ConfirmSpec {
        id: "cb_delete".into(),
        title: "Delete".into(),
        headline,
        total: items.len(),
        items,
        consequence,
        action: "Delete".into(),
        destructive: true,
    }
}

/// Asks, then deletes the selection for good through `content_delete`.
pub(crate) fn delete_selection(
    ctx: &egui::Context,
    editor: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    st: &Inner,
    has_project: bool,
) {
    if st.sel.is_empty() {
        return;
    }
    if st.loc.source.read_only() || !has_project {
        gz_ui::state::toast(ctx, gz_ui::Tone::Info, "This source is read-only: nothing here is deleted.");
        return;
    }
    let doomed: Vec<Entry> = st.sel.entries().to_vec();
    let cat = browser.cats.peek((Source::Project, 0));
    let spec = delete_spec(&doomed, cat.as_deref());
    let project = st.scope.0.clone();
    let ed = editor.clone();
    let inbox = browser.inbox.clone();
    let line = dialog::confirm(ctx, asker(editor), spec, move |answer| {
        // the answer comes a frame later: the project it was asked about must still be the open one
        if answer != Answer::Yes
            || ed.project_root().is_none_or(|r| !r.display().to_string().eq_ignore_ascii_case(&project))
        {
            return;
        }
        let list = doomed.iter().map(|e| e.path.to_string()).collect::<Vec<_>>().join(";");
        inbox.lock().clear_selection = true;
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            match call(&ed2, "content_delete", &[("path", json!(list)), ("recursive", json!(true))]).await {
                Ok(r) => ed2.log.info(format!(
                    "Deleted {} file(s), {} folder(s) for good.",
                    count(&r, "files"),
                    count(&r, "folders")
                )),
                Err((_, r)) => ed2.notices.error(format!(
                    "Deleted {} file(s); {} could not be deleted: {}",
                    count(&r, "files"),
                    count(&r, "failed"),
                    first_reason(&r)
                )),
            }
            ed2.exec.wake().ping();
        });
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

// ---- the clipboard ------------------------------------------------------------------------------

/// The clipboard entries of `entries`: a project folder goes as a folder (the copier walks it); a
/// game folder, which is no folder on disk, is the files under it, each keeping its place below
/// the folder's own name (`ClipboardCopy`, `ClipExpandFolder`).
#[must_use]
pub(crate) fn clips_of(entries: &[Entry], cat: &Catalog) -> Vec<Clip> {
    let mut out = Vec::new();
    for e in entries {
        let leaf = leaf_of(&e.path);
        if e.folder && cat.disk_root.is_none() {
            for f in cat.files_below(&e.path) {
                let below = f.path.get(e.path.len() + 1..).unwrap_or("");
                out.push(Clip { src: f.path.to_string(), rel: format!("{leaf}\\{below}"), abs: None });
            }
            continue;
        }
        out.push(Clip { src: e.path.to_string(), rel: leaf.to_owned(), abs: cat.disk_path(&e.path) });
    }
    out
}

pub(crate) fn clipboard_copy(
    ctx: &egui::Context,
    editor: &Editor,
    st: &mut Inner,
    cat: Option<&Catalog>,
    cut: bool,
) {
    if st.sel.is_empty() {
        return;
    }
    if cut && st.loc.source.read_only() {
        gz_ui::state::toast(ctx, gz_ui::Tone::Info, "This source is read-only: Cut moves things, use Copy.");
        return;
    }
    let Some(cat) = cat else { return };
    let n = st.sel.len();
    st.clip = Clipboard { source: st.loc.source, cut, items: clips_of(st.sel.entries(), cat) };
    editor.log.info(format!(
        "{} {n} entr{}.",
        if cut { "Cut" } else { "Copied" },
        if n == 1 { "y" } else { "ies" }
    ));
}

/// Why a paste cannot happen here, or `None`.
#[must_use]
pub(crate) fn paste_blocked(st: &Inner) -> Option<&'static str> {
    if st.clip.items.is_empty() {
        Some("The clipboard is empty.")
    } else if st.loc.source != Source::Project {
        Some("This source is read-only: paste into the project.")
    } else {
        None
    }
}

/// Copies or moves clipboard entries into the project folder `dst` (project-relative): the
/// entries grouped by the folder each lands in, one command per group. Answers the files
/// written and the first failure.
pub(crate) async fn transfer(
    editor: &Arc<Editor>,
    source: Source,
    cut: bool,
    clips: Vec<Clip>,
    dst: &str,
) -> (u64, Option<String>) {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for c in clips {
        let sub = parent_of(&c.rel);
        let folder = match (dst.is_empty(), sub.is_empty()) {
            (true, _) => sub.to_owned(),
            (false, true) => dst.to_owned(),
            (false, false) => format!("{dst}\\{sub}"),
        };
        let src = match source {
            Source::Project => c.src,
            // the library is on disk: its files go by their full path
            Source::Editor => c.abs.map_or(c.src, |p| p.display().to_string()),
            // a gamedata-relative path, which the copier finds in the game's archives
            Source::Game => c.src,
        };
        match groups.iter_mut().find(|(f, _)| f.eq_ignore_ascii_case(&folder)) {
            Some((_, list)) => list.push(src),
            None => groups.push((folder, vec![src])),
        }
    }
    let (mut files, mut first_error, mut failed) = (0u64, None::<String>, 0u64);
    for (folder, list) in groups {
        let dst_arg = if folder.is_empty() { ".".to_owned() } else { folder };
        let cmd = if cut && source == Source::Project { "content_move" } else { "content_copy" };
        let pairs = [
            ("src", json!(list.join(";"))),
            ("dst", json!(dst_arg)),
            ("recursive", json!(true)),
            ("overwrite", json!(true)),
        ];
        match call(editor, cmd, &pairs).await {
            Ok(r) => files += count(&r, "files"),
            Err((e, r)) => {
                files += count(&r, "files");
                failed += count(&r, "failed").max(1);
                first_error.get_or_insert_with(|| {
                    let reason = first_reason(&r);
                    if reason.is_empty() { e } else { reason }
                });
            }
        }
    }
    (files, first_error.map(|e| format!("{failed} failed, first: {e}")))
}

/// Paste into the folder shown; what would be replaced is asked about first (`ClipboardPaste`).
pub(crate) fn paste(
    ctx: &egui::Context,
    editor: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    st: &mut Inner,
) {
    if paste_blocked(st).is_some() {
        return;
    }
    let dst = st.loc.folder.clone();
    let project = browser.cats.peek((Source::Project, 0));
    let clashes: Vec<String> = st
        .clip
        .items
        .iter()
        .filter(|c| {
            let target = if dst.is_empty() { c.rel.clone() } else { format!("{dst}\\{}", c.rel) };
            project.as_ref().is_some_and(|p| p.file(&target).is_some() || p.folder(&target).is_some())
        })
        .map(|c| c.rel.clone())
        .collect();
    let clip = st.clip.clone();
    if clip.cut {
        // a cut is spent once it has been pasted, like everywhere else
        st.clip = Clipboard::default();
    }
    let ed = editor.clone();
    let run = move || {
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            let (files, err) = transfer(&ed2, clip.source, clip.cut, clip.items, &dst).await;
            match err {
                None => ed2.log.info(format!("Pasted {files} file(s).")),
                Some(e) => ed2.notices.error(format!("Pasted {files} file(s); {e}")),
            }
            ed2.exec.wake().ping();
        });
    };
    if clashes.is_empty() {
        run();
        return;
    }
    let where_ =
        if st.loc.folder.is_empty() { "the project's root".to_owned() } else { st.loc.folder.clone() };
    let spec = ConfirmSpec {
        id: "cb_replace".into(),
        title: "Replace".into(),
        headline: format!("{} item(s) already exist in {where_} and will be replaced:", clashes.len()),
        total: clashes.len(),
        items: clashes,
        consequence: "What they replace is gone for good - this cannot be undone.".into(),
        action: "Replace".into(),
        destructive: true,
    };
    // -nodlg answers No, so an unattended run never silently replaces files
    let line = dialog::confirm(ctx, asker(editor), spec, move |answer| {
        if answer == Answer::Yes {
            run();
        }
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

/// A drop of tiles onto a project folder: project entries move there, read-only ones are copied.
pub(crate) fn drop_into(
    editor: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    payload: &crate::DragPayload,
    category: usize,
    dst: &str,
) {
    let key: CatKey = (payload.source, if payload.source == Source::Editor { category } else { 0 });
    let Some(cat) = browser.cats.peek(key) else { return };
    // a folder never goes into itself or below itself
    let entries: Vec<Entry> = payload
        .entries
        .iter()
        .filter(|e| {
            payload.source != Source::Project
                || !(crate::model::is_ancestor(&e.path, dst) || parent_of(&e.path).eq_ignore_ascii_case(dst))
        })
        .map(|e| Entry::new(&e.path, e.folder))
        .collect();
    if entries.is_empty() {
        return;
    }
    let clips = clips_of(&entries, &cat);
    let cut = payload.source == Source::Project;
    let ed = editor.clone();
    let dst = dst.to_owned();
    editor.exec.spawn(async move {
        let (files, err) = transfer(&ed, key.0, cut, clips, &dst).await;
        let verb = if cut { "Moved" } else { "Copied" };
        match err {
            None => ed.log.info(format!(
                "{verb} {files} file(s) into {}.",
                if dst.is_empty() { "the project's root" } else { &dst }
            )),
            Some(e) => ed.notices.error(format!("{verb} {files} file(s); {e}")),
        }
        ed.exec.wake().ping();
    });
}

// ---- making things ------------------------------------------------------------------------------

/// The first free "New Folder", "New Folder 2", ... in a folder of the project's catalog.
#[must_use]
pub(crate) fn free_folder_name(cat: Option<&Catalog>, folder: &str) -> Option<String> {
    let node = cat.and_then(|c| c.folder(folder));
    let taken = |leaf: &str| {
        node.is_some_and(|n| {
            n.files.iter().any(|f| leaf_of(&f.path).eq_ignore_ascii_case(leaf))
                || n.children
                    .iter()
                    .filter_map(|&i| cat.and_then(|c| c.node(i)))
                    .any(|c| c.name.eq_ignore_ascii_case(leaf))
        })
    };
    (1..=999)
        .map(|n| if n == 1 { "New Folder".to_owned() } else { format!("New Folder {n}") })
        .find(|leaf| !taken(leaf))
}

/// New Folder: made, then its name is up for typing.
pub(crate) fn new_folder(editor: &Arc<Editor>, browser: &Arc<ContentBrowser>, st: &Inner) {
    let cat = browser.cats.peek((Source::Project, 0));
    let Some(leaf) = free_folder_name(cat.as_deref(), &st.loc.folder) else {
        editor.notices.error("Too many unnamed folders here.");
        return;
    };
    let rel = if st.loc.folder.is_empty() { leaf } else { format!("{}\\{leaf}", st.loc.folder) };
    let inbox = browser.inbox.clone();
    let ed = editor.clone();
    let cats = browser.cats.clone();
    editor.exec.spawn(async move {
        match call(&ed, "content_mkdir", &[("path", json!(rel))]).await {
            Ok(_) => {
                // the tile has to exist before it can be renamed
                let _ = cats.fresh(&ed, (Source::Project, 0)).await;
                inbox.lock().rename = Some(Entry::new(&rel, true));
            }
            Err((e, _)) => ed.notices.error(format!("Cannot create the folder: {e}")),
        }
        ed.exec.wake().ping();
    });
}

/// Add > Quest, Behaviour Graph, Character, PDA Interface: `content_create` in the folder shown,
/// then its name is up for typing.
pub(crate) fn create(editor: &Arc<Editor>, browser: &Arc<ContentBrowser>, st: &Inner, kind: &'static str) {
    let folder = st.loc.folder.clone();
    let inbox = browser.inbox.clone();
    let ed = editor.clone();
    let cats = browser.cats.clone();
    editor.exec.spawn(async move {
        let mut pairs = vec![("kind", json!(kind))];
        if !folder.is_empty() {
            pairs.push(("folder", json!(folder)));
        }
        match call(&ed, "content_create", &pairs).await {
            Ok(r) => {
                let made = r.get("made").and_then(Value::as_str).unwrap_or("").replace('/', "\\");
                let _ = cats.fresh(&ed, (Source::Project, 0)).await;
                if !made.is_empty() {
                    inbox.lock().rename = Some(Entry::new(&made, false));
                }
            }
            Err((e, _)) => ed.notices.error(e),
        }
        ed.exec.wake().ping();
    });
}

/// The Add menu's items: the label, the tip, the `content_create` kind (`None`: a folder).
pub(crate) const ADD_ITEMS: [(&str, &str, Option<&str>); 5] = [
    ("New Folder", "A folder in the folder shown.", None),
    ("Quest", "A quest graph (.nqasset).", Some("quest")),
    ("Behaviour Graph", "What an NPC does, step by step (.behasset).", Some("behaviour")),
    ("Character", "One of the module's own NPCs (.npcasset).", Some("character")),
    (
        "PDA Interface",
        "The module's own PDA screens: pages, buttons, menus, the live map (.pdaasset).",
        Some("pda"),
    ),
];

// ---- Exclude from Build ---------------------------------------------------------------------------

/// What the build-contents row of the menu offers for a selection (`DrawBuildToggle`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BuildToggle {
    /// Nothing of it ships ever: the row is off.
    Never,
    Exclude(Vec<String>),
    Include(Vec<String>),
    /// Another rule takes it out: include that one instead.
    ByRule(String),
}

#[must_use]
pub(crate) fn build_toggle(rules: &[String], entries: &[Entry]) -> BuildToggle {
    use gz_mod::exclude::{ShipState, ship_state};
    let (mut shipped, mut own, mut never) = (Vec::new(), Vec::new(), 0usize);
    let mut by = String::new();
    for e in entries {
        match ship_state(rules, &e.path) {
            ShipState::Shipped => shipped.push(e.path.to_string()),
            ShipState::Excluded => own.push(e.path.to_string()),
            ShipState::ExcludedByRule(rule) => by = rule,
            ShipState::Never => never += 1,
        }
    }
    if never > 0 && shipped.is_empty() && own.is_empty() && by.is_empty() {
        BuildToggle::Never
    } else if !shipped.is_empty() {
        BuildToggle::Exclude(shipped)
    } else if !own.is_empty() && by.is_empty() {
        BuildToggle::Include(own)
    } else {
        BuildToggle::ByRule(by)
    }
}

/// Exclude from Build / Include in Build through `mod_exclude`, one entry after another.
pub(crate) fn set_excluded(editor: &Arc<Editor>, paths: Vec<String>, excluded: bool) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        for p in paths {
            if let Err((e, _)) =
                call(&ed, "mod_exclude", &[("path", json!(p)), ("excluded", json!(excluded))]).await
            {
                ed.notices.error(format!("Build Contents: {e}"));
                break;
            }
        }
        ed.exec.wake().ping();
    });
}

// ---- outside the editor ---------------------------------------------------------------------------

/// Show in Explorer: the entry selected in a window of Explorer; `dry_run` only answers the path
/// (`RevealInExplorer`).
pub(crate) fn reveal_in_explorer(
    project_root: Option<PathBuf>,
    entry: &str,
    dry_run: bool,
) -> Result<String, String> {
    let Some(root) = project_root else { return Err("no project is open".into()) };
    if entry.is_empty() {
        return Err("missing 'name'".into());
    }
    let rel = crate::model::normalize_folder(&crate::model::normalize_name(entry)).map_err(str::to_owned)?;
    if rel.is_empty() {
        return Err("name the entry to select, not the project root".into());
    }
    let path = format!("{}\\{rel}", root.display());
    if std::fs::symlink_metadata(&path).is_err() {
        return Err("no such file or folder in the project".into());
    }
    if dry_run {
        return Ok(path);
    }
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{path}"))
        .spawn()
        .map_err(|_| "Explorer did not start".to_owned())?;
    Ok(path)
}

/// Link Game... of the Game source's empty state: the folder picker on a thread of its own, then
/// `game_link`.
pub(crate) fn link_game(editor: &Arc<Editor>) {
    let ed = editor.clone();
    let asker = if editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
    let request = gz_ui::file_dialog::FileRequest::folder("Link Game");
    let picked = gz_ui::file_dialog::pick(None, asker, request, move |folder| {
        let path = folder.display().to_string();
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            if let Err((e, _)) = call(&ed2, "game_link", &[("path", json!(path))]).await {
                ed2.notices.error(format!("Link Game: {e}"));
            }
            ed2.exec.wake().ping();
        });
    });
    if let Err(line) = picked {
        editor.log.plain(line);
    }
}

/// Place in Scene: the scene's `place_object` with the entry's library reference.
pub(crate) fn place(editor: &Arc<Editor>, reference: String) {
    editor.post("place_object", args(&[("ref", json!(reference))]));
}

// ---- reaching the browser -------------------------------------------------------------------------

/// Brings the browser up from the window: the panel while Level draws it, the Content Drawer over
/// anything else (`MakeVisible`).
pub(crate) fn make_visible(cx: &mut DocCx<'_>, st: &Inner) {
    let now = cx.ctx.cumulative_frame_nr();
    if cx.active_tab == gz_app::tabs::LEVEL && st.host(crate::Host::Panel).recent(now) {
        cx.requests.panel(PANEL_ID, PanelOp::Focus);
    } else {
        cx.requests.drawer(Some(Drawer::Content));
    }
}

/// The tab menu's Show in Content Browser: the project file of a tab selected in the browser.
pub(crate) fn reveal_from_tab(cx: &mut DocCx<'_>, path: &str) {
    let browser = ContentBrowser::of(cx.editor);
    let mut st = browser.inner.lock();
    let has_project = cx.editor.project_opt().is_some();
    let rel = crate::model::normalize_name(path);
    let folder = parent_of(&rel).to_owned();
    let origin = st.loc.clone();
    let target = Location::new(Source::Project, st.loc.category, &folder);
    if st.apply_location(target, false, &browser.cats, has_project).is_ok() {
        st.remember(&origin);
        st.search.clear();
        st.sel.only(Entry::new(&rel, false));
        st.scroll_to_selection = true;
        st.pics.cancel();
    }
    make_visible(cx, &st);
}

/// An Editor category's files that back an entry, mirrored into the project's layout
/// (`CopyLibraryItem` without a destination): `(from, project folder)` pairs that exist.
#[must_use]
pub(crate) fn library_files(editor_root: &std::path::Path, cat: usize, path: &str) -> Vec<(PathBuf, String)> {
    let c = category(cat);
    let stem = icons::extension(path).map_or(path, |e| &path[..path.len() - e.len()]);
    let mut out = Vec::new();
    for lib in c.lib {
        let from = editor_root.join(lib.folder).join(format!("{stem}{}", lib.ext));
        if from.is_file() {
            let sub = parent_of(stem);
            let dst = if sub.is_empty() { lib.folder.to_owned() } else { format!("{}\\{sub}", lib.folder) };
            out.push((from, dst));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_double_click_opens() {
        assert_eq!(visual_name("meshes\\dynamics\\box\\box_1a.ogf"), "dynamics\\box\\box_1a");
        assert_eq!(game_level(Source::Game, "levels\\l05_bar\\level.ai"), Some("l05_bar"));
        assert_eq!(game_level(Source::Project, "levels\\x"), None);
        assert!(
            matches!(what_opens(Source::Game, 0, "levels\\l05_bar", None), Ok(Open::Scene(s)) if s == "l05_bar")
        );
        let Ok(Open::Doc(q)) = what_opens(Source::Project, 0, "quests\\a.nqasset", None) else { panic!() };
        assert_eq!((q.kind.as_str(), q.path.as_deref()), ("quest", Some("quests\\a.nqasset")));
        assert!(what_opens(Source::Game, 0, "x.nqasset", None).is_err());
        let Ok(Open::Doc(t)) = what_opens(Source::Game, 0, "textures\\ui\\x.dds", None) else { panic!() };
        assert_eq!((t.kind.as_str(), t.arg.as_deref()), ("texture", Some("darf")));
        assert!(what_opens(Source::Project, 0, "notes.txt", None).is_err_and(|e| e.starts_with("no viewer")));
    }

    #[test]
    fn references_and_names() {
        assert_eq!(
            placeable_ref(Source::Project, 0, "rawdata\\objects\\box\\b.object").as_deref(),
            Some("box\\b")
        );
        assert_eq!(placeable_ref(Source::Project, 0, "gamedata\\b.object"), None);
        assert_eq!(placeable_ref(Source::Editor, 0, "box\\b.object").as_deref(), Some("box\\b"));
        let spec = delete_spec(&[Entry::new("a\\b.txt", false)], None);
        assert_eq!(spec.headline, "Delete b.txt?");
        let spec = delete_spec(&[Entry::new("a", true), Entry::new("b.txt", false)], None);
        assert_eq!(spec.headline, "Delete 1 folder and 1 file?");
        assert!(spec.destructive && spec.items == ["a\\", "b.txt"]);
        assert_eq!(free_folder_name(None, ""), Some("New Folder".into()));
    }

    #[test]
    fn the_build_toggle() {
        let rules = vec!["docs".to_owned()];
        assert_eq!(
            build_toggle(&rules, &[Entry::new("docs", true)]),
            BuildToggle::Include(vec!["docs".into()])
        );
        assert_eq!(
            build_toggle(&rules, &[Entry::new("docs\\a.md", false)]),
            BuildToggle::ByRule("docs".into())
        );
        assert_eq!(
            build_toggle(&rules, &[Entry::new("gamedata", true)]),
            BuildToggle::Exclude(vec!["gamedata".into()])
        );
    }
}
