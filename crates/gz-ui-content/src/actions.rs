//! What the browser does: every action is an editor command (`file.rename`, `file.move`,
//! `file.copy`, `file.delete`, `file.mkdir`, `file.create`, `file.import`, `mod.exclude`,
//! `ui.tab`, `scene.open`, `scene.place`), so the window and MCP share one code path. The browser
//! changes no file itself, but for a picture from Explorer made into a texture of the project
//! (`import_files`).

use std::path::PathBuf;
use std::sync::Arc;

use gz_app::{DocCx, OpenRequest};
use gz_editor::{Args, Editor, Value, json};
use gz_ui::dialog::{self, Answer, Asker, ConfirmSpec};
use gz_ui::{Tx, tr};

use crate::catalog::{Catalog, Origin};
use crate::icons;
use crate::model::{self, Location, Source, leaf_of, parent_of};
use crate::select::Entry;
use crate::service::{CatKey, Clip, Clipboard, ContentBrowser, ImportClash, Inner, Rename};

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
    /// A base scene of the library: brought into the project, then opened there.
    BaseScene(String),
    /// A level of the game: the project's scene over it, or a new one.
    LevelScene(String),
}

/// What opening an entry means. `cat` is the catalog it is listed in.
pub(crate) fn what_opens(cat: &Catalog, path: &str) -> Result<Open, String> {
    if path.is_empty() {
        return Err("empty asset name".into());
    }
    let ext = icons::extension(path).map(str::to_ascii_lowercase).unwrap_or_default();
    if cat.source == Source::Library {
        return match cat.category {
            // a level of the game opens the project's scene over it; a base scene of the
            // library comes into the project, then opens there
            model::SCENES => Ok(match crate::library::game_level(cat, path) {
                Some(level) => Open::LevelScene(level),
                None => Open::BaseScene(crate::catalog::stem(path).to_owned()),
            }),
            model::OBJECTS => Ok(Open::Doc(OpenRequest {
                arg: Some("library".to_owned()),
                ..OpenRequest::file("model", path)
            })),
            model::TEXTURES => {
                let (name, source) = match cat.file(path).map(|f| f.origin) {
                    Some(Origin::Game) => (cat.game_path(path).unwrap_or_else(|| path.to_owned()), "game"),
                    _ => (crate::catalog::stem(path).to_owned(), "library"),
                };
                Ok(Open::Doc(OpenRequest {
                    arg: Some(source.to_owned()),
                    ..OpenRequest::file("texture", &name)
                }))
            }
            _ => Err(format!("no viewer for this asset kind yet: '{path}'")),
        };
    }
    // a scene (or an X-Ray SDK level, converted as it opens) opens in the Level tab
    if ext == ".scene" || ext == ".level" {
        let scene = cat.disk_path(path).map_or_else(|| path.to_owned(), |p| p.display().to_string());
        return Ok(Open::Scene(scene));
    }
    if ext == ".ogf" || ext == ".object" {
        // an object of the project's library folder opens by its reference, as a scene names it
        let lower = path.to_ascii_lowercase();
        let name =
            if ext == ".object" && lower.starts_with("rawdata\\objects\\") { &path[16..] } else { path };
        return Ok(Open::Doc(OpenRequest {
            arg: Some("project".to_owned()),
            ..OpenRequest::file("model", name)
        }));
    }
    match ext.as_str() {
        // a model asset in the model viewer, a texture asset in the texture preview, as the game
        // and the level compiler will have them; a sound asset in the Sound tab
        ".meshasset" => {
            return Ok(Open::Doc(OpenRequest {
                arg: Some("project".to_owned()),
                ..OpenRequest::file("model", path)
            }));
        }
        ".texasset" => {
            return Ok(Open::Doc(OpenRequest {
                arg: Some("project".to_owned()),
                ..OpenRequest::file("texture", path)
            }));
        }
        ".sndasset" => return Ok(Open::Doc(OpenRequest::file("sound", path))),
        ".pdaasset" => return Ok(Open::Doc(OpenRequest::file("pda", path))),
        ".npcasset" => return Ok(Open::Doc(OpenRequest::file("characters", path))),
        ".itemasset" => return Ok(Open::Doc(OpenRequest::file("items", path))),
        ".tradeasset" | ".lootasset" | ".recipeasset" => {
            return Ok(Open::Doc(OpenRequest::file("economy", path)));
        }
        ".nqasset" | ".behasset" => {
            return Ok(Open::Doc(OpenRequest::file(if ext == ".nqasset" { "quest" } else { "beh" }, path)));
        }
        _ => {}
    }
    if icons::is_image(path) {
        return Ok(Open::Doc(OpenRequest {
            arg: Some("project".to_owned()),
            ..OpenRequest::file("texture", path)
        }));
    }
    // a script, a config, XML, Markdown...: the Text tab; a file of another name is looked into
    // off the UI thread first (`open_if_text`)
    if gz_text::is_text_name(path) {
        return Ok(Open::Doc(OpenRequest::file("text", path)));
    }
    Err(format!("no viewer for this asset kind yet: '{path}'"))
}

/// Whether a file of the project reads as text (no NUL in its first bytes, not too large): what
/// opens in the Text tab when its name says nothing. On the IO pool.
pub(crate) async fn is_text_file(editor: &Arc<Editor>, path: &str) -> Result<(), String> {
    use std::io::Read as _;
    let Some(session) = editor.project_opt() else { return Err("no project is open".to_owned()) };
    let rel = gz_base::RelPath::new(path).map_err(|_| format!("{path} is no file of the project"))?;
    let abs = session.fs(&rel);
    let shown = path.to_owned();
    editor
        .exec
        .io(move || -> Result<(), String> {
            let meta = std::fs::metadata(&abs)
                .map_err(|e| format!("{shown} could not be read: {}", gz_base::io_reason(&e)))?;
            if meta.len() > gz_text::MAX_BYTES {
                return Err(format!("{shown}: {}", gz_text::ReadError::TooLarge(meta.len())));
            }
            let mut head = vec![0u8; 8192];
            let n = std::fs::File::open(&abs)
                .and_then(|mut f| f.read(&mut head))
                .map_err(|e| format!("{shown} could not be read: {}", gz_base::io_reason(&e)))?;
            if gz_text::sniff(&head[..n]) {
                Ok(())
            } else {
                Err(format!("{shown}: {}", gz_text::ReadError::NotText))
            }
        })
        .await
}

/// Open as Text, and a double click on a file whose name says nothing: the Text tab when the file
/// is text, a note saying why not otherwise.
pub(crate) fn open_as_text(editor: &Arc<Editor>, path: &str) {
    let (ed, path) = (editor.clone(), path.to_owned());
    editor.exec.spawn(async move {
        match is_text_file(&ed, &path).await {
            Ok(()) => {
                if let Err((e, _)) = call(
                    &ed,
                    "ui.tab",
                    &[("action", json!("open")), ("kind", json!("text")), ("path", json!(path))],
                )
                .await
                {
                    ed.notices.error(e);
                }
            }
            Err(e) => ed.notices.note(format!("No viewer for this kind of entry yet: {e}")),
        }
    });
}

/// A double click from the window: into a folder, an asset's tab.
pub(crate) fn open_entry(
    cx: &mut DocCx<'_>,
    st: &mut Inner,
    browser: &ContentBrowser,
    entry: &Entry,
    has_project: bool,
) {
    let source = st.loc.source;
    if entry.folder {
        // into the folder a search found: the search has done its job
        st.search.clear();
        let target = if st.loc.home() {
            Location::new(Source::Library, model::find_kind(&entry.path).unwrap_or(model::HOME), "")
        } else {
            Location::new(source, st.loc.category, &entry.path)
        };
        if st.apply_location(target, true, &browser.cats, has_project).is_ok() {
            st.pics.cancel();
        }
        return;
    }
    st.sel.only(entry.clone());
    let Some(cat) = browser.cats.peek(crate::service::cat_key(&st.loc)) else { return };
    match what_opens(&cat, &entry.path) {
        Ok(Open::Doc(req)) => cx.requests.open(req),
        Ok(Open::Scene(scene)) => open_scene(cx.editor, SceneOpen::File(scene)),
        Ok(Open::BaseScene(name)) => open_scene(cx.editor, SceneOpen::Base(name)),
        Ok(Open::LevelScene(level)) => open_scene(cx.editor, SceneOpen::Over(level)),
        // a file whose name says nothing may still be text
        Err(e) if e.starts_with("no viewer") && source == Source::Project && has_project => {
            open_as_text(cx.editor, &entry.path);
        }
        Err(e) if e.starts_with("no viewer") => {
            cx.editor.notices.note(format!("No viewer for this kind of entry yet: '{}'", entry.path))
        }
        Err(e) => cx.editor.notices.error(e),
    }
}

/// The same for MCP: the outcome comes back instead of a notice.
pub(crate) async fn open_for_mcp(editor: &Arc<Editor>, cat: &Catalog, path: &str) -> Result<(), String> {
    let opens = match what_opens(cat, path) {
        // a file whose name says nothing opens as text when it is text
        Err(e) if e.starts_with("no viewer") && cat.source == Source::Project => {
            is_text_file(editor, path).await?;
            Open::Doc(OpenRequest::file("text", path))
        }
        other => other?,
    };
    match opens {
        Open::Doc(req) => {
            let mut pairs = vec![("action", json!("open")), ("kind", json!(req.kind))];
            if let Some(p) = &req.path {
                pairs.push(("path", json!(p)));
            }
            if let Some(s) = &req.arg {
                pairs.push(("source", json!(s)));
            }
            call(editor, "ui.tab", &pairs).await.map(|_| ()).map_err(|(e, _)| format!("{path}: {e}"))
        }
        Open::Scene(scene) => {
            call(editor, "scene.open", &[("path", json!(scene))]).await.map(|_| ()).map_err(|(e, _)| e)
        }
        Open::BaseScene(name) => {
            let scene = bring_in_base_scene(editor, &name).await?;
            call(editor, "scene.open", &[("path", json!(scene))]).await.map(|_| ()).map_err(|(e, _)| e)
        }
        Open::LevelScene(level) => {
            scene_over_level(editor, &level, &[]).await.map(|_| ()).map_err(String::from)
        }
    }
}

/// What the browser opens in the Level tab.
pub(crate) enum SceneOpen {
    /// A scene file of the project.
    File(String),
    /// A base scene of the library, by its name.
    Base(String),
    /// The project's scene over a level of the game.
    Over(String),
}

/// Opens a scene from the window: unsaved work of the open scene is asked about (Save / Don't
/// Save / Cancel) and the open goes on with the answer; the Level tab comes up with the scene.
pub(crate) fn open_scene(editor: &Arc<Editor>, what: SceneOpen) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let first = run_scene_open(&ed, &what, &[]).await;
        let reply = match first {
            Ok(()) => return show_level(&ed),
            Err((why, reply)) => {
                if reply.get("code").and_then(Value::as_str) != Some("unsaved") {
                    return ed.notices.error(why);
                }
                reply
            }
        };
        // the scene in the way, by its name
        let named: Vec<gz_ui::Tx<'static>> = reply
            .get("unsaved")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|u| u.get("name").and_then(Value::as_str))
            .map(|n| gz_ui::Tx::raw(n.to_owned()))
            .collect();
        let named = if named.is_empty() { vec![gz_ui::Tx::from("Untitled scene")] } else { named };
        // the question is a kit dialog: it is asked on the UI thread, where the dialogs live and
        // where who is asking is known - asked from this task it could be answered for a command
        // that happened to run at that moment, or come up as a sheet of another dialog
        let Ok(queue) = gz_app::UiQueue::of(&ed) else { return };
        let ed1 = ed.clone();
        queue.post(move |_shell, ctx| {
            let ed2 = ed1.clone();
            let line = dialog::unsaved_prompt(ctx, asker(&ed1), "scene", &named, move |answer| {
                let flag = match answer {
                    dialog::Answer::Yes => "save",
                    dialog::Answer::No => "discard",
                    dialog::Answer::Cancel => return,
                };
                let ed3 = ed2.clone();
                ed2.exec.spawn(async move {
                    match run_scene_open(&ed3, &what, &[(flag, json!(true))]).await {
                        Ok(()) => show_level(&ed3),
                        Err((why, _)) => ed3.notices.error(why),
                    }
                });
            });
            if let Some(l) = line {
                ed1.log.finding(&l);
            }
        });
    });
}

fn show_level(editor: &Arc<Editor>) {
    editor.post("ui.tab", args(&[("action", json!("activate")), ("doc", json!("level"))]));
}

/// One open of [`SceneOpen`], with what settles unsaved work (`save` or `discard`).
async fn run_scene_open(
    editor: &Arc<Editor>,
    what: &SceneOpen,
    settle: &[(&str, Value)],
) -> Result<(), (String, Value)> {
    let with = |path: String| {
        let mut pairs = vec![("path", json!(path))];
        pairs.extend(settle.iter().cloned());
        pairs
    };
    match what {
        SceneOpen::File(file) => call(editor, "scene.open", &with(file.clone())).await.map(|_| ()),
        SceneOpen::Base(name) => {
            let scene = bring_in_base_scene(editor, name).await.map_err(|e| (e, Value::Null))?;
            call(editor, "scene.open", &with(scene)).await.map(|_| ())
        }
        SceneOpen::Over(level) => {
            scene_over_level(editor, level, settle).await.map(|_| ()).map_err(|e| match e {
                OverError::Refused(why, reply) => (why, reply),
                OverError::Other(why) => (why, Value::Null),
            })
        }
    }
}

/// Why the project's scene over a level did not open.
pub(crate) enum OverError {
    /// A command refused: its sentence and its answer (`code: unsaved` among them).
    Refused(String, Value),
    Other(String),
}

impl From<OverError> for String {
    fn from(e: OverError) -> Self {
        match e {
            OverError::Refused(why, _) | OverError::Other(why) => why,
        }
    }
}

/// The project's scene over a level of the game: the scene of the project that says
/// `base = "<level>"` is opened; without one a new scene over the level is made and saved as
/// `rawdata\levels\<level>.scene` (the first free name). Answers the scene's file.
pub(crate) async fn scene_over_level(
    editor: &Arc<Editor>,
    level: &str,
    settle: &[(&str, Value)],
) -> Result<String, OverError> {
    let Ok((_, project)) = gz_project::Project::of(editor) else {
        return Err(OverError::Other("no project is open".to_owned()));
    };
    project.tree.refresh().await;
    let root = std::path::PathBuf::from(project.root());
    let scenes: Vec<std::path::PathBuf> =
        project.tree.snapshot().files_with_extension("scene").iter().map(|r| r.to_fs(&root)).collect();
    let wanted = level.to_owned();
    let (found, unread) = editor
        .exec
        .io(move || {
            let mut unread = Vec::new();
            let found = scenes.into_iter().find(|file| {
                // a scene that does not read says nothing of the level it lies over
                if gz_scene::format::level_of_readable_file(file).is_none() {
                    unread.push(file.clone());
                    return false;
                }
                gz_scene::format::base_of_file(file).is_some_and(|b| b.eq_ignore_ascii_case(&wanted))
            });
            (found, unread)
        })
        .await;
    // a second scene over the level would bake into the same files as the first: while one of
    // the project's scenes does not read, none is made
    if found.is_none()
        && let Some(file) = unread.first()
    {
        let rel = file.strip_prefix(&root).unwrap_or(file).display().to_string().replace('\\', "/");
        return Err(OverError::Other(format!(
            "{rel} does not read, so nobody can say whether it is the project's scene over {level}: mend it (asset.check path={rel}) and open the level again."
        )));
    }
    let refused = |(why, reply): (String, Value)| OverError::Refused(why, reply);
    if let Some(file) = found {
        let file = file.display().to_string();
        let mut pairs = vec![("path", json!(file))];
        pairs.extend(settle.iter().cloned());
        call(editor, "scene.open", &pairs).await.map_err(refused)?;
        return Ok(file);
    }
    let mut pairs = vec![("base", json!(level))];
    pairs.extend(settle.iter().cloned());
    call(editor, "scene.new", &pairs).await.map_err(refused)?;
    // where the project keeps its scenes, under the first name nobody has
    let dir = root.join("rawdata").join("levels");
    let file = (1..)
        .map(|n| if n == 1 { format!("{level}.scene") } else { format!("{level}_{n}.scene") })
        .map(|name| dir.join(name))
        .find(|f| {
            std::fs::symlink_metadata(f).is_err() && std::fs::symlink_metadata(f.with_extension("")).is_err()
        })
        .unwrap_or_else(|| dir.join(format!("{level}.scene")));
    let file = file.display().to_string();
    call(editor, "scene.save", &[("path", json!(file))]).await.map_err(refused)?;
    project.tree.mark_fs(&dir);
    Ok(file)
}

/// A base scene of the library copied into the project (a scene the project has already is the
/// author's copy, left as it is); answers the project's scene file.
pub(crate) async fn bring_in_base_scene(editor: &Arc<Editor>, name: &str) -> Result<String, String> {
    let Some(project) = editor.project_root() else { return Err("no project is open".to_owned()) };
    let root = gz_project::library_root(editor);
    let n = name.to_owned();
    let made = editor.exec.io(move || gz_project::project::import_base_scene(&root, &project, &n)).await?;
    if let Ok((_, p)) = gz_project::Project::of(editor)
        && let Some(dir) = made.parent()
    {
        p.tree.mark_fs(dir);
    }
    Ok(made.display().to_string())
}

/// What a tile places in a level, as `scene.place` takes it: `(kind, field, value)` - the field
/// is `ref`, or `section` for a spawn. `None` for what is not placed.
#[must_use]
pub(crate) fn placeable(cat: &Catalog, path: &str) -> Option<(String, &'static str, String)> {
    let asset = crate::payload_of(cat, path, false);
    let p = asset.path;
    match asset.kind.as_str() {
        // a model asset by its project path, with its extension
        "object" if gz_project::asset_kind::MediaKind::of_file(&p).is_some() => Some((asset.kind, "ref", p)),
        // an object by its reference: below `rawdata\objects`, without the extension
        "object" => {
            let below =
                if p.to_ascii_lowercase().starts_with("rawdata\\objects\\") { &p[16..] } else { p.as_str() };
            Some((asset.kind, "ref", crate::catalog::stem(below).to_owned()))
        }
        // a group of the project by its file, one of the library by its reference
        "group" if cat.source == Source::Library => {
            Some((asset.kind, "ref", crate::catalog::stem(&p).to_owned()))
        }
        "group" | "sound" | "particles" => Some((asset.kind, "ref", p)),
        "spawn" => Some((asset.kind, "section", p)),
        _ => None,
    }
}

// ---- the inline rename --------------------------------------------------------------------------

/// Opens the rename box on an entry: a folder is its whole name; one of the editor's own assets
/// shows its name without the extension; any other file keeps its extension out of the selection
///.
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

/// The box is done: `file.rename` with what it holds, the asset's companions renamed with it. The browser follows a
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
    let companions: Vec<String> = browser
        .cats
        .peek((Source::Project, 0))
        .and_then(|c| c.file(&was).map(|f| f.companions.iter().map(ToString::to_string).collect()))
        .unwrap_or_default();
    let inbox = browser.inbox.clone();
    let ed = editor.clone();
    editor.exec.spawn(async move {
        // a media asset that names its source keeps it as it is named: the name is in its text
        let named = names_its_source(&ed, &was).await;
        let companions: Vec<String> =
            companions.into_iter().filter(|c| follows_rename(&was, c, named)).collect();
        match call(&ed, "file.rename", &[("path", json!(was)), ("name", json!(r.text))]).await {
            Ok(reply) => {
                // the thumbnail and the scene's folder take the new name too
                let new_stem =
                    crate::catalog::stem(leaf_of(reply.get("path").and_then(Value::as_str).unwrap_or("")))
                        .to_owned();
                let old_stem = crate::catalog::stem(leaf_of(&was)).to_owned();
                for c in &companions {
                    let leaf = leaf_of(c);
                    let renamed = format!("{new_stem}{}", leaf.get(old_stem.len()..).unwrap_or(""));
                    if let Err((e, _)) =
                        call(&ed, "file.rename", &[("path", json!(c)), ("name", json!(renamed))]).await
                    {
                        ed.log.warn(format!("'{c}' kept its name: {e}"));
                    }
                }
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

/// Whether a companion takes the asset's new name: a thumbnail and a scene's folder always do;
/// what a media asset takes its data from only when it is found by the asset's own name (the
/// asset names no `source`) and has that name.
#[must_use]
pub(crate) fn follows_rename(asset: &str, companion: &str, names_source: bool) -> bool {
    if gz_project::asset_kind::MediaKind::of_file(asset).is_none() {
        return true;
    }
    !names_source
        && crate::catalog::stem(leaf_of(companion)).eq_ignore_ascii_case(crate::catalog::stem(leaf_of(asset)))
}

/// Whether the media asset at `rel` (project-relative) names its source in its text.
async fn names_its_source(editor: &Arc<Editor>, rel: &str) -> bool {
    let Some(kind) = gz_project::asset_kind::MediaKind::of_file(rel) else { return false };
    let Some(root) = editor.project_root() else { return false };
    let rel = rel.replace('\\', "/");
    editor
        .exec
        .io(move || {
            let text = std::fs::read(root.join(&rel)).ok()?;
            let value = gz_media::kind::load(kind, &rel, &String::from_utf8_lossy(&text)).ok()?;
            value.get("source").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty())
        })
        .await
        .unwrap_or(false)
}

// ---- delete ------------------------------------------------------------------------------------

/// The confirmation of a deletion: it names what goes.
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
        [one] if one.folder => {
            tr!("Delete the folder {name} and everything in it?", name = leaf_of(&one.path))
        }
        [one] => tr!("Delete {name}?", name = leaf_of(&one.path)),
        _ if folders > 0 && files > 0 => tr!(
            "Delete {folders} {folders:folder|folders} and {files} {files:file|files}?",
            folders = folders,
            files = files
        ),
        _ if folders > 0 => tr!("Delete {n} {n:folder|folders}?", n = folders),
        _ => tr!("Delete {n} {n:file|files}?", n = files),
    };
    let consequence = match (inside, folders) {
        (0, _) => Tx::EMPTY,
        (_, 1) => tr!("It holds {n} {n:file|files}. Deleted for good - this cannot be undone.", n = inside),
        _ => {
            tr!("The folders hold {n} {n:file|files}. Deleted for good - this cannot be undone.", n = inside)
        }
    };
    ConfirmSpec {
        id: "cb_delete".into(),
        // a title names the deletion; the button that does it is the verb
        title: Tx::ctx("dlg.cb_delete", "Delete"),
        headline,
        total: items.len(),
        items: items.into_iter().map(Tx::raw).collect(),
        consequence,
        action: "Delete".into(),
        destructive: true,
    }
}

/// Asks, then deletes the selection for good through `file.delete`, every asset with its
/// companions.
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
        let list = with_companions(&doomed, cat.as_deref());
        inbox.lock().clear_selection = true;
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            match call(&ed2, "file.delete", &[("path", json!(list)), ("recursive", json!(true))]).await {
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
        editor.log.finding(&line);
    }
}

/// The paths of entries with what goes with each of them (a thumbnail, a scene's folder).
#[must_use]
pub(crate) fn with_companions(entries: &[Entry], cat: Option<&Catalog>) -> Vec<String> {
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        out.push(e.path.to_string());
        if !e.folder
            && let Some(f) = cat.and_then(|c| c.file(&e.path))
        {
            out.extend(f.companions.iter().map(ToString::to_string));
        }
    }
    out
}

// ---- the clipboard ------------------------------------------------------------------------------

/// One file's clip: the project's by its path, the Library's by where its bytes are - a file of
/// the library on disk, or a game file (`gamedata`-relative); an entry without a file has none.
fn clip_of(cat: &Catalog, f: &crate::catalog::FileItem, rel: String) -> Option<Clip> {
    match (cat.source, f.origin) {
        (Source::Project, _) => Some(Clip { src: f.path.to_string(), rel }),
        (Source::Library, Origin::Disk) => {
            let abs = cat.disk_path(&f.path)?;
            Some(Clip { src: abs.display().to_string(), rel })
        }
        (Source::Library, Origin::Game) => Some(Clip { src: cat.game_path(&f.path)?, rel }),
        (Source::Library, Origin::None) => None,
    }
}

/// The clipboard entries of `entries`, each asset with its companions: a project folder goes as a
/// folder (the copier walks it); a Library folder is the files under it, each keeping its place
/// below the folder's own name.
#[must_use]
pub(crate) fn clips_of(entries: &[Entry], cat: &Catalog) -> Vec<Clip> {
    let mut out = Vec::new();
    for e in entries {
        let leaf = leaf_of(&e.path);
        if e.folder {
            if cat.source == Source::Project {
                out.push(Clip { src: e.path.to_string(), rel: leaf.to_owned() });
            } else {
                for f in cat.files_below(&e.path) {
                    let below = f.path.get(e.path.len() + 1..).unwrap_or("");
                    out.extend(clip_of(cat, f, format!("{leaf}\\{below}")));
                    for c in &f.companions {
                        let below = c.get(e.path.len() + 1..).unwrap_or("");
                        let cf = crate::catalog::FileItem { path: c.clone(), ..f.clone() };
                        out.extend(clip_of(cat, &cf, format!("{leaf}\\{below}")));
                    }
                }
            }
            continue;
        }
        let Some(f) = cat.file(&e.path) else { continue };
        out.extend(clip_of(cat, f, leaf.to_owned()));
        for c in &f.companions {
            let cf = crate::catalog::FileItem { path: c.clone(), ..f.clone() };
            out.extend(clip_of(cat, &cf, below_folder_of(&e.path, c)));
        }
    }
    out
}

/// Where a companion lies below its asset's folder (a glTF's `textures\a.png`): it keeps that
/// place wherever the asset goes. One outside the folder goes beside it.
#[must_use]
pub(crate) fn below_folder_of(asset: &str, companion: &str) -> String {
    let dir = parent_of(asset);
    if dir.is_empty() {
        return companion.to_owned();
    }
    match companion.get(..=dir.len()) {
        Some(head) if head.eq_ignore_ascii_case(&format!("{dir}\\")) => companion[dir.len() + 1..].to_owned(),
        _ => leaf_of(companion).to_owned(),
    }
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
        // the project's by its path, the library's by its full path, the game's gamedata-relative
        // (the copier finds it in the game's archives)
        let src = c.src;
        match groups.iter_mut().find(|(f, _)| f.eq_ignore_ascii_case(&folder)) {
            Some((_, list)) => list.push(src),
            None => groups.push((folder, vec![src])),
        }
    }
    let (mut files, mut first_error, mut failed) = (0u64, None::<String>, 0u64);
    for (folder, list) in groups {
        let dst_arg = if folder.is_empty() { ".".to_owned() } else { folder };
        let cmd = if cut && source == Source::Project { "file.move" } else { "file.copy" };
        let pairs = [
            ("path", json!(list)),
            ("to", json!(dst_arg)),
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

/// Paste into the folder shown; what would be replaced is asked about first.
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
    let where_ = if st.loc.folder.is_empty() {
        Tx::ctx("in_folder", "the project's root")
    } else {
        Tx::raw(st.loc.folder.clone())
    };
    let spec = ConfirmSpec {
        id: "cb_replace".into(),
        title: Tx::ctx("dlg.cb_replace", "Replace"),
        headline: tr!(
            "{n} {n:item already exists|items already exist} in {folder} and will be replaced:",
            n = clashes.len(),
            folder = where_
        ),
        total: clashes.len(),
        items: clashes.into_iter().map(Tx::raw).collect(),
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
        editor.log.finding(&line);
    }
}

/// A drop of the browser's own tiles onto a project folder: project entries move there, the
/// Library's are copied in (Bring into Project into that folder).
pub(crate) fn drop_into(
    editor: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    from: &crate::service::Dragging,
    dst: &str,
) {
    let key: CatKey = from.key;
    let Some(cat) = browser.cats.peek(key) else { return };
    // a folder never goes into itself or below itself
    let entries: Vec<Entry> = from
        .entries
        .iter()
        .filter(|e| {
            key.0 != Source::Project
                || !(crate::model::is_ancestor(&e.path, dst) || parent_of(&e.path).eq_ignore_ascii_case(dst))
        })
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }
    let clips = clips_of(&entries, &cat);
    let cut = key.0 == Source::Project;
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

/// Pictures the game does not read (PNG, JPG, TGA, BMP).
fn convertible_picture(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg", "tga", "bmp"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Whether a project folder keeps the files brought into it as they are: the game's own tree
/// (`gamedata`) and the X-Ray SDK's library (`rawdata`), whose files the game and the editor read
/// by where they lie and what they are.
#[must_use]
pub(crate) fn keeps_files_as_they_are(dst: &str) -> bool {
    let top = dst.split(['\\', '/']).next().unwrap_or("");
    top.eq_ignore_ascii_case("gamedata") || top.eq_ignore_ascii_case("rawdata")
}

/// What the files brought into `dst` are made into: the models, pictures and sounds that become
/// assets (`asset.import`), and the rest, copied as they are.
#[must_use]
pub(crate) fn import_split(files: Vec<PathBuf>, dst: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
    if keeps_files_as_they_are(dst) {
        return (Vec::new(), files);
    }
    files.into_iter().partition(|f| {
        f.file_name().is_some_and(|n| gz_media::files::kind_of_source(&n.to_string_lossy()).is_some())
    })
}

/// Files from Explorer dropped on a project folder, or picked with Add > Import...: a model
/// (`.glb`, `.gltf`, `.obj`, `.object`), a picture or a sound becomes an asset there
/// (`asset.import`: the file copied in, its asset beside it); anything else is copied in
/// (`file.copy`). The game's and the X-Ray SDK's folders keep what comes in as it is, but for a
/// picture dropped under `gamedata\textures`, which becomes a `.dds` there, the way the game reads
/// it. The frame says how many assets were made; files of the project in the way are asked about
/// first.
pub(crate) fn import_files(
    editor: &Arc<Editor>,
    browser: &Arc<ContentBrowser>,
    files: Vec<PathBuf>,
    dst: &str,
) {
    let ed = editor.clone();
    let inbox = browser.inbox.clone();
    let dst = dst.to_owned();
    editor.exec.spawn(async move {
        let (media, files) = import_split(files, &dst);
        let textures = dst.to_ascii_lowercase().starts_with("gamedata\\textures");
        let (pictures, plain): (Vec<PathBuf>, Vec<PathBuf>) =
            files.into_iter().partition(|f| textures && convertible_picture(f));
        let mut done = 0u64;
        let mut errors: Vec<String> = Vec::new();
        if !media.is_empty() {
            let from: Vec<String> = media.iter().map(|p| p.display().to_string()).collect();
            match import_assets(&ed, &from, &dst, false).await {
                Ok(made) => done += made,
                Err(Imported::Clash(there)) => {
                    inbox.lock().import_clash = Some(ImportClash { from, to: dst.clone(), there });
                }
                Err(Imported::Failed(e)) => errors.push(e),
            }
        }
        if !plain.is_empty() {
            let list: Vec<String> = plain.iter().map(|p| p.display().to_string()).collect();
            let to = if dst.is_empty() { ".".to_owned() } else { dst.clone() };
            match call(
                &ed,
                "file.copy",
                &[("path", json!(list)), ("to", json!(to)), ("recursive", json!(true))],
            )
            .await
            {
                Ok(r) => done += count(&r, "files"),
                Err((e, r)) => {
                    done += count(&r, "files");
                    let reason = first_reason(&r);
                    errors.push(if reason.is_empty() { e } else { reason });
                }
            }
        }
        if !pictures.is_empty() {
            match gz_project::Project::of(&ed) {
                Ok((_, project)) => {
                    let root = PathBuf::from(project.root());
                    let guard = project.write_guard(None);
                    let folder = root.join(&dst);
                    let made = ed
                        .exec
                        .io(move || {
                            let mut made = 0u64;
                            let mut failed = Vec::new();
                            for f in pictures {
                                match picture_to_dds(&f, &folder, &guard) {
                                    Ok(()) => made += 1,
                                    Err(e) => failed.push(e),
                                }
                            }
                            (made, failed)
                        })
                        .await;
                    done += made.0;
                    errors.extend(made.1);
                    project.tree.mark_fs(&root.join(&dst));
                }
                Err(e) => errors.push(e.message),
            }
        }
        match errors.first() {
            None => ed.log.info(format!(
                "Imported {done} file(s) into {}.",
                if dst.is_empty() { "the project's root" } else { &dst }
            )),
            Some(e) => {
                ed.notices.error(format!("Imported {done} file(s); {} failed, first: {e}", errors.len()))
            }
        }
        ed.exec.wake().ping();
    });
}

/// Why an import made nothing.
pub(crate) enum Imported {
    /// Files of the project are in the way: these.
    Clash(Vec<String>),
    Failed(String),
}

/// `asset.import` of `from` into the project folder `to` (the kinds' own folders at the root);
/// answers the files it wrote. The assets made are the frame's toast; what the import left out
/// and why is a notice.
pub(crate) async fn import_assets(
    editor: &Arc<Editor>,
    from: &[String],
    to: &str,
    overwrite: bool,
) -> Result<u64, Imported> {
    let mut pairs = vec![("from", json!(from))];
    if !to.is_empty() {
        pairs.push(("to", json!(to)));
    }
    if overwrite {
        pairs.push(("overwrite", json!(true)));
    }
    let browser = ContentBrowser::of(editor);
    let list = |r: &Value, key: &str| -> Vec<String> {
        r.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    match call(editor, "asset.import", &pairs).await {
        Ok(r) => {
            let (assets, copied, problems) = (list(&r, "assets"), list(&r, "copied"), list(&r, "problems"));
            if !problems.is_empty() {
                editor.notices.push(gz_editor::NoticeKind::Warning, problems.join("\n"));
            }
            browser.inbox.lock().imported = Some(assets.len() as u64);
            editor.exec.wake().ping();
            Ok((assets.len() + copied.len()) as u64)
        }
        Err((_, r)) if r.get("code").and_then(Value::as_str) == Some("exists") => {
            Err(Imported::Clash(list(&r, "files")))
        }
        Err((e, r)) => {
            let problems = list(&r, "problems");
            Err(Imported::Failed(if problems.is_empty() {
                e
            } else {
                format!("{e}: {}", problems.join("; "))
            }))
        }
    }
}

/// The question of an import whose files would replace the project's: it names them.
#[must_use]
pub(crate) fn replace_spec(there: &[String]) -> ConfirmSpec {
    ConfirmSpec {
        id: "cb_import_replace".into(),
        title: Tx::ctx("dlg.cb_import_replace", "Import"),
        headline: tr!(
            "{n} {n:file is|files are} in the project already and will be replaced:",
            n = there.len()
        ),
        total: there.len(),
        items: there.iter().map(|t| Tx::raw(t.clone())).collect(),
        consequence: "What they replace is gone for good - this cannot be undone.".into(),
        action: "Replace".into(),
        destructive: true,
    }
}

/// An import the frame found files in the way of: the author is asked, and Replace sends it
/// again with `overwrite=true`. Under `-nodlg` the answer is No and nothing is replaced.
pub(crate) fn ask_import_clash(ctx: &egui::Context, editor: &Arc<Editor>, clash: ImportClash) {
    let spec = replace_spec(&clash.there);
    let ed = editor.clone();
    let line = dialog::confirm(ctx, asker(editor), spec, move |answer| {
        if answer != Answer::Yes {
            return;
        }
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            match import_assets(&ed2, &clash.from, &clash.to, true).await {
                Ok(_) => {}
                Err(Imported::Clash(t)) => {
                    ed2.notices.error(format!("Cannot import: {} files are still in the way.", t.len()));
                }
                Err(Imported::Failed(e)) => ed2.notices.error(format!("Cannot import: {e}")),
            }
            ed2.exec.wake().ping();
        });
    });
    if let Some(line) = line {
        editor.log.finding(&line);
    }
}

/// Add > Import...: the system's file dialog, then the file it picked is brought into the folder
/// shown as a drop of it would be.
pub(crate) fn import_dialog(editor: &Arc<Editor>, browser: &Arc<ContentBrowser>, folder: &str) {
    let request = gz_ui::file_dialog::FileRequest::open("Import")
        .filter(
            "Models, pictures and sounds",
            &["glb", "gltf", "obj", "object", "png", "jpg", "jpeg", "tga", "bmp", "dds", "wav", "ogg"],
        )
        .filter("All files", &["*"]);
    let (ed, b, dst) = (editor.clone(), browser.clone(), folder.to_owned());
    let picked = gz_ui::file_dialog::pick(&editor.exec, None, asker(editor), request, move |file| {
        import_files(&ed, &b, vec![file], &dst);
    });
    if let Err(line) = picked {
        editor.log.finding(&line);
    }
}

/// One picture made into `<folder>\<name>.dds` with a game texture's defaults.
fn picture_to_dds(
    file: &std::path::Path,
    folder: &std::path::Path,
    guard: &gz_editor::fsx::WriteGuard,
) -> Result<(), String> {
    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let target = folder.join(format!("{name}.dds"));
    guard.check(&target).map_err(|e| e.to_string())?;
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    let bytes = std::fs::read(file).map_err(|e| format!("{} cannot be read: {e}", file.display()))?;
    let loaded = gz_fmt_tex::load_image(&bytes, ext.as_deref())
        .map_err(|e| format!("{} is no picture the editor reads: {e}", file.display()))?;
    let params = gz_fmt_tex::TextureParams {
        width: i32::try_from(loaded.image.width).unwrap_or(0),
        height: i32::try_from(loaded.image.height).unwrap_or(0),
        ..gz_fmt_tex::TextureParams::default()
    };
    let made = gz_fmt_tex::make_game_texture(&loaded.image, &params, None).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    gz_editor::fsx::write_atomic(&target, &made.dds)
        .map_err(|e| format!("{} cannot be written: {e}", target.display()))
}

/// Bring into Project: Library entries copied into the project where a scene finds them - an
/// object with its textures and thumbnail (`file.import`), a texture or a sound of the game or
/// the library under the project's `gamedata`, a group under `rawdata\groups`, a base scene
/// under `rawdata\levels`.
pub(crate) async fn bring_into_project(
    editor: &Arc<Editor>,
    cat: &Catalog,
    entries: &[Entry],
) -> (u64, Vec<String>) {
    let (mut done, mut errors) = (0u64, Vec::new());
    let Some(k) = model::kind(cat.category).filter(|_| cat.source == Source::Library) else {
        return (0, vec!["only the Library's entries are brought into the project".to_owned()]);
    };
    let mut files: Vec<&crate::catalog::FileItem> = Vec::new();
    for e in entries {
        if e.folder {
            files.extend(cat.files_below(&e.path));
        } else if let Some(f) = cat.file(&e.path) {
            files.push(f);
        }
    }
    for f in files {
        let result = match (cat.category, f.origin) {
            // a level of the game is not copied: the project's scene lies over it
            (model::SCENES, Origin::Game) => Err(format!(
                "{} is a level of the game: Open starts the project's scene over it",
                crate::catalog::stem(model::leaf_of(&f.path))
            )),
            (model::SCENES, _) => {
                bring_in_base_scene(editor, crate::catalog::stem(&f.path)).await.map(|_| ())
            }
            (_, Origin::None) => {
                Err(format!("{} has no file of its own: it is placed, not brought in", f.path))
            }
            (_, Origin::Game) => {
                let path = cat.game_path(&f.path).unwrap_or_default();
                call(editor, "file.import", &[("from", json!("game")), ("path", json!(path))])
                    .await
                    .map(|_| ())
                    .map_err(|(e, _)| e)
            }
            (_, Origin::Disk) => {
                let path = format!("{}\\{}", k.library_folder, f.path);
                let mut result =
                    call(editor, "file.import", &[("from", json!("library")), ("path", json!(path))])
                        .await
                        .map(|_| ())
                        .map_err(|(e, _)| e);
                // an object's file.import brings its thumbnail along; anything else takes its own
                if cat.category != model::OBJECTS && result.is_ok() {
                    for c in &f.companions {
                        let path = format!("{}\\{c}", k.library_folder);
                        if let Err((e, _)) =
                            call(editor, "file.import", &[("from", json!("library")), ("path", json!(path))])
                                .await
                        {
                            result = Err(e);
                        }
                    }
                }
                result
            }
        };
        match result {
            Ok(()) => done += 1,
            Err(e) if e.contains("in the project already") => done += 1,
            Err(e) => errors.push(e),
        }
    }
    (done, errors)
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
        match call(&ed, "file.mkdir", &[("path", json!(rel))]).await {
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

/// Add > Quest, Behaviour Graph, Character, Item, Trader, Loot Table, Recipes, PDA Interface:
/// `file.create` in the folder shown, then its name is up for typing.
pub(crate) fn create(editor: &Arc<Editor>, browser: &Arc<ContentBrowser>, st: &Inner, kind: &'static str) {
    let folder = st.loc.folder.clone();
    let inbox = browser.inbox.clone();
    let ed = editor.clone();
    let cats = browser.cats.clone();
    editor.exec.spawn(async move {
        let mut pairs = vec![("kind", json!(kind))];
        if !folder.is_empty() {
            pairs.push(("path", json!(folder)));
        }
        match call(&ed, "file.create", &pairs).await {
            Ok(r) => {
                let made = r.get("path").and_then(Value::as_str).unwrap_or("").replace('/', "\\");
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

/// The Add menu's Import...: not a `file.create` kind.
pub(crate) const IMPORT: &str = "import";

/// The Add menu's items: the label, the tip, the `file.create` kind (`None`: a folder; [`IMPORT`]:
/// the file dialog).
pub(crate) const ADD_ITEMS: [(&str, &str, Option<&str>); 10] = [
    ("New Folder", "A folder in the folder shown.", None),
    (
        "Import...",
        "Models, pictures and sounds become assets in the folder shown; any other file is copied in.",
        Some(IMPORT),
    ),
    ("Quest", "A quest graph (.nqasset).", Some("quest")),
    ("Behaviour Graph", "What an NPC does, step by step (.behasset).", Some("behaviour")),
    ("Character", "One of the module's own NPCs (.npcasset).", Some("character")),
    ("Item", "An inventory item of the module, or a change of one of the game's (.itemasset).", Some("item")),
    ("Trader", "What the module puts on the shelf of a trader of the game (.tradeasset).", Some("trader")),
    ("Loot Table", "What the dead carry, and how often (.lootasset).", Some("loot")),
    ("Recipes", "What is made at a tool or a workbench, and from what (.recipeasset).", Some("recipes")),
    (
        "PDA Interface",
        "The module's own PDA screens: pages, buttons, menus, the live map (.pdaasset).",
        Some("pda"),
    ),
];

// ---- Exclude from Build ---------------------------------------------------------------------------

/// What the build-contents row of the menu offers for a selection.
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

/// Exclude from Build / Include in Build through `mod.exclude`, one entry after another.
pub(crate) fn set_excluded(editor: &Arc<Editor>, paths: Vec<String>, excluded: bool) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        for p in paths {
            if let Err((e, _)) =
                call(&ed, "mod.exclude", &[("path", json!(p)), ("on", json!(excluded))]).await
            {
                ed.notices.error(format!("Build Contents: {e}"));
                break;
            }
        }
        ed.exec.wake().ping();
    });
}

// ---- outside the editor ---------------------------------------------------------------------------

/// Show in Explorer: the entry selected in a window of Explorer; `dry_run` only answers the path.
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

/// Link Game...: Preferences > Game, where the default game is picked.
pub(crate) fn link_game(editor: &Arc<Editor>) {
    editor.post("app.prefs", args(&[("action", json!("show")), ("page", json!("game"))]));
}

/// Place in Level: what the entry is, on the surface under the viewport's centre - the viewport's
/// own drop (`viewport.gesture`) at the middle of its rectangle, the snapping that is on included,
/// one undo step, the placed thing selected. Where the Level tab is not up, `scene.place` puts it
/// on the surface below a point ten metres along the camera's view.
pub(crate) fn place(
    editor: &Arc<Editor>,
    asset: gz_ui::drop::Asset,
    (kind, field, value): (String, &'static str, String),
) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        if let Some(at) = viewport_centre(&ed).await {
            let assets = json!([{ "path": asset.path, "kind": asset.kind }]);
            let pairs = [("kind", json!("drop")), ("at", json!(at)), ("assets", assets)];
            if call(&ed, "viewport.gesture", &pairs).await.is_ok() {
                ed.exec.wake().ping();
                return;
            }
        }
        let mut pairs = vec![("kind", json!(kind)), (field, json!(value)), ("on_surface", json!(true))];
        if let Ok(cam) = call(&ed, "viewport.camera", &[]).await {
            let v = |k: &str| -> Option<[f64; 3]> {
                let a = cam.get(k)?.as_array()?;
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
            };
            if let (Some(p), Some(d)) = (v("position"), v("direction")) {
                pairs.push(("position", json!([p[0] + d[0] * 10.0, p[1] + d[1] * 10.0, p[2] + d[2] * 10.0])));
            }
        }
        if let Err((e, _)) = call(&ed, "scene.place", &pairs).await {
            ed.notices.error(format!("Place in Level: {e}"));
        }
        ed.exec.wake().ping();
    });
}

/// The middle of the viewport, in points from its top left, while the Level tab shows it.
async fn viewport_centre(editor: &Arc<Editor>) -> Option<[f64; 2]> {
    let list = call(editor, "ui.list", &[]).await.ok()?;
    let panel = list
        .get("panels")?
        .as_array()?
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some("viewport"))?;
    if panel.get("visible").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let r = panel.get("rect")?.as_array()?;
    let (w, h) = (r.get(2)?.as_f64()?, r.get(3)?.as_f64()?);
    (w > 1.0 && h > 1.0).then_some([(w * 0.5).round(), (h * 0.5).round()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_double_click_opens() {
        let mut project = Catalog::empty(Source::Project, 0);
        project.disk_root = Some(PathBuf::from("D:\\p"));
        for p in ["quests\\a.nqasset", "levels\\e.scene", "notes.txt", "gamedata\\textures\\x.dds"] {
            project.add(crate::catalog::FileItem {
                path: p.into(),
                size: 1,
                asset: gz_project::kind_of(p, None),
                origin: Origin::Disk,
                label: None,
                note: None,
                companions: Vec::new(),
                published: None,
            });
        }
        let Ok(Open::Doc(q)) = what_opens(&project, "quests\\a.nqasset") else { panic!() };
        assert_eq!((q.kind.as_str(), q.path.as_deref()), ("quest", Some("quests\\a.nqasset")));
        assert!(
            matches!(what_opens(&project, "levels\\e.scene"), Ok(Open::Scene(s)) if s.ends_with("e.scene"))
        );
        // a text file opens in the Text tab; a file whose name says nothing is looked into first
        let Ok(Open::Doc(t)) = what_opens(&project, "notes.txt") else { panic!() };
        assert_eq!((t.kind.as_str(), t.path.as_deref()), ("text", Some("notes.txt")));
        let Ok(Open::Doc(s)) = what_opens(&project, "scripts\\a.script") else { panic!() };
        assert_eq!(s.kind, "text");
        assert!(what_opens(&project, "data.bin").is_err_and(|e| e.starts_with("no viewer")));
        let mut tex = Catalog::empty(Source::Library, model::TEXTURES);
        tex.game_prefix = "textures".into();
        tex.add(crate::catalog::FileItem {
            path: "ui\\x.dds".into(),
            size: 1,
            asset: None,
            origin: Origin::Game,
            label: None,
            note: None,
            companions: Vec::new(),
            published: None,
        });
        let Ok(Open::Doc(t)) = what_opens(&tex, "ui\\x.dds") else { panic!() };
        assert_eq!(
            (t.kind.as_str(), t.arg.as_deref(), t.path.as_deref()),
            ("texture", Some("game"), Some("textures\\ui\\x.dds"))
        );
        let scenes = Catalog::empty(Source::Library, model::SCENES);
        assert!(matches!(what_opens(&scenes, "jupiter.level"), Ok(Open::BaseScene(n)) if n == "jupiter"));
    }

    #[test]
    fn what_is_placed_and_how_it_drags() {
        let mut objects = Catalog::empty(Source::Library, model::OBJECTS);
        objects.disk_root = Some(PathBuf::from("D:\\lib\\rawdata\\objects"));
        objects.add(crate::catalog::FileItem {
            path: "box\\b.object".into(),
            size: 1,
            asset: None,
            origin: Origin::Disk,
            label: None,
            note: None,
            companions: Vec::new(),
            published: None,
        });
        assert_eq!(placeable(&objects, "box\\b.object"), Some(("object".into(), "ref", "box\\b".into())));
        let a = crate::payload_of(&objects, "box\\b.object", false);
        assert_eq!((a.kind.as_str(), a.path.as_str()), ("object", "box\\b.object"));
        assert!(a.file.is_some_and(|f| f.ends_with("b.object")));
        let mut spawns = Catalog::empty(Source::Library, model::SPAWNS);
        spawns.add(crate::catalog::FileItem {
            path: "weapons\\pistols\\wpn_pm".into(),
            size: 0,
            asset: None,
            origin: Origin::None,
            label: Some("PM".into()),
            note: None,
            companions: Vec::new(),
            published: None,
        });
        assert_eq!(
            placeable(&spawns, "weapons\\pistols\\wpn_pm"),
            Some(("spawn".into(), "section", "wpn_pm".into()))
        );
        let a = crate::payload_of(&spawns, "weapons\\pistols\\wpn_pm", false);
        assert_eq!((a.kind.as_str(), a.path.as_str()), ("spawn", "wpn_pm"));
        let mut project = Catalog::empty(Source::Project, 0);
        project.add(crate::catalog::FileItem {
            path: "rawdata\\objects\\box\\b.object".into(),
            size: 1,
            asset: None,
            origin: Origin::Disk,
            label: None,
            note: None,
            companions: Vec::new(),
            published: None,
        });
        assert_eq!(
            placeable(&project, "rawdata\\objects\\box\\b.object").map(|p| p.2),
            Some("box\\b".into())
        );
        let spec = delete_spec(&[Entry::new("a\\b.txt", false)], None);
        assert_eq!(spec.headline.en(), "Delete b.txt?");
        let spec = delete_spec(&[Entry::new("a", true), Entry::new("b.txt", false)], None);
        assert_eq!(spec.headline.en(), "Delete 1 folder and 1 file?");
        assert!(spec.destructive && spec.items == [Tx::raw("a\\"), Tx::raw("b.txt")]);
        assert_eq!(free_folder_name(None, ""), Some("New Folder".into()));
    }

    #[test]
    fn media_assets_open_place_rename_move_and_come_in() {
        let mut project = Catalog::empty(Source::Project, 0);
        project.disk_root = Some(PathBuf::from("D:\\p"));
        for p in ["models\\wall.meshasset", "textures\\brick.texasset", "sounds\\hum.sndasset"] {
            project.add(crate::catalog::FileItem {
                path: p.into(),
                size: 1,
                asset: gz_project::kind_of(p, None),
                origin: Origin::Disk,
                label: None,
                note: None,
                companions: Vec::new(),
                published: p.ends_with(".texasset").then(|| "bunker\\brick".into()),
            });
        }
        let Ok(Open::Doc(m)) = what_opens(&project, "models\\wall.meshasset") else { panic!() };
        assert_eq!((m.kind.as_str(), m.arg.as_deref()), ("model", Some("project")));
        let Ok(Open::Doc(t)) = what_opens(&project, "textures\\brick.texasset") else { panic!() };
        assert_eq!((t.kind.as_str(), t.path.as_deref()), ("texture", Some("textures\\brick.texasset")));
        assert_eq!(
            what_opens(&project, "sounds\\hum.sndasset"),
            Ok(Open::Doc(OpenRequest::file("sound", "sounds\\hum.sndasset")))
        );
        // a model asset is placed by its project path, its extension kept
        assert_eq!(
            placeable(&project, "models\\wall.meshasset"),
            Some(("object".into(), "ref", "models\\wall.meshasset".into()))
        );
        assert!(placeable(&project, "textures\\brick.texasset").is_none());
        let a = crate::payload_of(&project, "textures\\brick.texasset", false);
        assert_eq!((a.kind.as_str(), a.path.as_str()), ("texture", "bunker\\brick"));
        let a = crate::payload_of(&project, "sounds\\hum.sndasset", false);
        assert_eq!((a.kind.as_str(), a.path.as_str()), ("sound", "sounds\\hum"));
        // a source found by the asset's name takes the new name; one the asset names keeps its own
        assert!(follows_rename("m\\wall.meshasset", "m\\wall.glb", false));
        assert!(!follows_rename("m\\wall.meshasset", "m\\wall.glb", true));
        assert!(!follows_rename("m\\wall.meshasset", "m\\wall_lod.bin", false));
        assert!(follows_rename("rawdata\\objects\\a.object", "rawdata\\objects\\a.thm", true));
        // what lies below the asset's folder keeps its place there
        assert_eq!(
            below_folder_of("models\\crate.meshasset", "models\\tex\\crate_d.png"),
            "tex\\crate_d.png"
        );
        assert_eq!(below_folder_of("models\\crate.meshasset", "MODELS\\crate.bin"), "crate.bin");
        assert_eq!(below_folder_of("models\\crate.meshasset", "other\\x.png"), "x.png");
        assert_eq!(below_folder_of("crate.meshasset", "tex\\a.png"), "tex\\a.png");
        // models, pictures and sounds become assets; the game's own folders keep them as they are
        let files = || {
            ["D:\\art\\wall.glb", "D:\\art\\brick.PNG", "D:\\art\\hum.wav", "D:\\art\\readme.txt"]
                .map(PathBuf::from)
                .to_vec()
        };
        let (media, rest) = import_split(files(), "models\\bunker");
        assert_eq!((media.len(), rest.len()), (3, 1));
        assert!(rest[0].ends_with("readme.txt"));
        let (media, rest) = import_split(files(), "gamedata\\textures\\ui");
        assert_eq!((media.len(), rest.len()), (0, 4));
        assert!(keeps_files_as_they_are("RawData") && !keeps_files_as_they_are("art\\gamedata"));
        let spec = replace_spec(&["models\\wall.glb".to_owned()]);
        assert!(spec.destructive && spec.items == [Tx::raw("models\\wall.glb")]);
        assert_eq!(spec.headline.en(), "1 file is in the project already and will be replaced:");
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
