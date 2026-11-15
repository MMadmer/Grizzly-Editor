//! The project area's commands: `project.*` (open, close, create, the list, delete, reveal, info,
//! guide, collect) and `file.*` (the project's files: list, copy, move, rename, delete, mkdir,
//! create from a template, import from the library or the game).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath};
use gz_editor::{CmdError, Editor, JsonSchema, ProjectSession, Registry, Reply, Spec};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::delete::{self, DeleteContext, DeleteInfo};
use crate::games::Machine;
use crate::hooks::{AssetLists, AssetTemplates, CATEGORIES, Lister, ProjectHooks, TemplateCx, TemplateKind};
use crate::legacy::{self, Legacy};
use crate::list::{self, EntryState};
use crate::machine::{self, ProjectState};
use crate::ops::{FileOps, Report};
use crate::project::{self, ProjectFile};
use crate::service::{Project, Projects};
use crate::{fsys, guide, library, paths, rules};
use gz_vfs::GameFiles;

/// The longest path a project folder may have.
const MAX_PATH: usize = 260;

pub fn register(reg: &mut Registry) {
    reg.area("project", "The project: open, close, create, the project list, delete, its library");
    reg.area("file", "The project's files: list, copy, move, rename, delete, create from a template, import");
    reg.cmd(
        Spec::read("project.list", "Lists the projects of the Start page with what each folder is now."),
        project_list,
    );
    reg.cmd(
        Spec::write("project.open", "Opens a project: its folder, its project file or its mod.ltx.")
            .destroys_with(&["discard"])
            .example("path=D:/Mods/NightWatch/NightWatch.grizzly"),
        project_open,
    );
    reg.cmd(
        Spec::write("project.close", "Closes the open project and goes back to the Start page.")
            .destroys_with(&["discard"]),
        project_close,
    );
    reg.cmd(
        Spec::write("project.create", "Makes a new project folder with its project file and opens it.")
            .example("name=NightWatch"),
        project_create,
    );
    reg.cmd(Spec::destroy("project.delete", "Deletes a listed project folder for good."), project_delete);
    reg.cmd(
        Spec::write("project.forget", "Takes a project whose folder is gone off the Start page's list."),
        project_forget,
    );
    reg.cmd(Spec::view("project.reveal", "Shows a listed project's folder in Explorer."), project_reveal);
    reg.cmd(
        Spec::read("project.info", "Says what the open project is: its file, id, game and library list."),
        project_info,
    );
    reg.cmd(
        Spec::write("project.guide", "Writes the project's AGENTS.md again from what it holds."),
        project_guide,
    );
    reg.cmd(
        Spec::write(
            "project.collect",
            "Copies everything the scenes take from the editor's library into the project.",
        )
        .example("dry_run=true"),
        project_collect,
    );
    reg.cmd(
        Spec::write(
            "project.set_library",
            "Sets the editor's library folder, where scenes take what the project lacks.",
        )
        .example("path=E:/Grizzly Library"),
        project_set_library,
    );
    reg.cmd(
        Spec::write(
            "project.associate",
            "Says or sets whether .grizzly files open with this editor, and registers it.",
        )
        .example("now=true"),
        project_associate,
    );
    reg.cmd(
        Spec::read("file.list", "Lists files of the project, the editor's library or the game.")
            .example("source=game kind=dds query=textures limit=20"),
        file_list,
    );
    reg.cmd(
        Spec::write("file.copy", "Copies files or folders of the project into a folder of it.")
            .destroys_with(&["overwrite"])
            .example("path=quests/a.nqasset to=quests/old"),
        file_copy,
    );
    reg.cmd(
        Spec::write("file.move", "Moves files or folders of the project into a folder of it.")
            .destroys_with(&["overwrite"]),
        file_move,
    );
    reg.cmd(Spec::destroy("file.delete", "Deletes files or folders of the project for good."), file_delete);
    reg.cmd(Spec::write("file.mkdir", "Makes a folder in the project."), file_mkdir);
    reg.cmd(
        Spec::write("file.rename", "Renames a file or folder of the project; open documents follow it.")
            .example("path=quests/a.nqasset name=night_watch"),
        file_rename,
    );
    reg.cmd(
        Spec::write("file.create", "Makes a new asset from its template in a folder of the project.")
            .example("kind=quest path=quests"),
        file_create,
    );
    reg.cmd(
        Spec::write("file.import", "Copies a file of the editor's library or of the game into the project.")
            .destroys_with(&["overwrite"])
            .example("from=game path=textures/ui/ui_icon.dds"),
        file_import,
    );
}

/// A refusal: a rule says no.
fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A refusal that carries fields of the reply after `error`.
fn refuse_with(message: impl Into<String>, fields: Map<String, Value>) -> CmdError {
    refuse(message).with_data(Value::Object(fields))
}

// ---- the project list -------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

async fn project_list(ed: Arc<Editor>, _p: NoArgs, _origin: Origin) -> Reply {
    let list = Projects::of(&ed).list(&ed).await;
    let entries = list.entries_with_ids();
    let states = ed
        .exec
        .io(move || entries.into_iter().map(|(p, id)| (list::entry_state(&p), p, id)).collect::<Vec<_>>())
        .await;
    let projects: Vec<Value> = states
        .iter()
        .map(|(state, path, id)| {
            json!({ "name": paths::wire(list::entry_name(path)), "path": paths::wire(path), "state": state.as_str(), "id": id })
        })
        .collect();
    Ok(json!({ "projects": projects }))
}

// ---- open, close, create ----------------------------------------------------------------------

/// Whether unsaved work was let go: `discard`, or the window's own action under `-nodlg`, whose
/// question has the logged answer No. A command's caller is never that: on any editor it says
/// `discard`, through the destroy door, or is refused with `unsaved`.
fn lets_go(discard: bool, origin: Origin) -> bool {
    discard || origin == Origin::Unattended
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenParams {
    /// The project: its folder, its project file (`*.grizzly`) or its `mod.ltx`. From the command
    /// line a listed project's name works too.
    path: String,
    /// Save the open project's unsaved work first.
    #[serde(default)]
    save: bool,
    /// Drop the open project's unsaved work.
    #[serde(default)]
    discard: bool,
    /// A second start of the editor handed the project over: unsaved work in the way is asked
    /// about in the window, as Switch Project asks.
    #[serde(default)]
    from_launch: bool,
    /// Bring up the scene the project was last working on. `false` opens the project alone: a
    /// commandlet that works on files does not wait for a level to load.
    #[serde(default)]
    scene: Option<bool>,
}

async fn project_open(ed: Arc<Editor>, p: OpenParams, origin: Origin) -> Reply {
    if p.save && p.discard {
        return Err(CmdError::invalid("save and discard exclude each other"));
    }
    let path = paths::to_backslash(p.path.trim());
    if path.is_empty() {
        return Err(CmdError::invalid("project.open needs the path of a project"));
    }
    let probe = PathBuf::from(&path);
    let folder = ed.exec.io(move || project::project_folder_of(&probe)).await;
    // -project <name> from the command line: the name of a listed project
    let folder = match folder {
        Some(f) => Some(f.display().to_string()),
        None if origin != Origin::Mcp && !paths::is_absolute(&path) => {
            let list = Projects::of(&ed).list(&ed).await;
            list.entries().into_iter().find(|e| list::entry_name(e).eq_ignore_ascii_case(&path))
        }
        None => None,
    };
    let Some(folder) = folder else {
        return Err(refuse(format!(
            "There is no project at {}: give its folder, its project file or its mod.ltx.",
            paths::wire(&path)
        )));
    };
    if let Ok((_, open)) = Project::of(&ed)
        && paths::same_path(open.root(), &folder)
    {
        return Ok(json!({ "note": "already open", "path": paths::wire(open.root()) }));
    }
    if ed.project_opt().is_some() && !p.save && !lets_go(p.discard, origin) && !ed.dirty_docs().is_empty() {
        // the question is a person's: an editor without one (no window, -nodlg) refuses like any
        // other open, and the unsaved work stays
        if p.from_launch
            && !ed.flags.headless
            && !ed.flags.unattended
            && ProjectHooks::of(&ed).ask_before_launch(&ed, &folder)
        {
            return Ok(json!({ "asking": true, "path": paths::wire(&folder) }));
        }
        ed.refuse_unsaved(false)?;
    }
    if p.save && ed.project_opt().is_some() {
        ed.save_all().await?;
        // what a save could not write is not dropped by the open that follows
        let left = ed.dirty_docs();
        if let Some(d) = left.first() {
            return Err(refuse_with(
                format!("{}: it is still unsaved", d.name),
                Map::from_iter([("dirty".to_owned(), serde_json::to_value(&left).unwrap_or(Value::Null))]),
            ));
        }
    }
    let discard = lets_go(p.discard, origin);
    let opened = open(&ed, &folder, discard, false, p.scene.unwrap_or(true)).await?;
    Ok(json!({ "path": paths::wire(&opened.root), "id": opened.id, "file": paths::wire(&opened.file) }))
}

/// What an open did.
pub(crate) struct Opened {
    pub root: String,
    pub id: String,
    /// The project file's name.
    pub file: String,
}

/// What the open reads and writes on the IO pool before the project is up.
struct Prepared {
    file_path: PathBuf,
    file: ProjectFile,
    state: ProjectState,
    /// The old `project.ltx` it converted, if any.
    legacy: Option<Legacy>,
    gitignore: std::io::Result<bool>,
    guide: Option<std::io::Result<bool>>,
    problems: Vec<String>,
    notes: Vec<String>,
}

/// The project file of the folder: the one there, or a new one for a folder with `mod.ltx` (a
/// fresh clone) or an old `project.ltx` (converted, then deleted). Blocking.
fn prepare(root: &Path, home: &Path, scaffolds: &[crate::hooks::ScaffoldHook]) -> Result<Prepared, String> {
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // the module skeleton first: a project that predates it gets it silently
    let mut problems: Vec<String> = scaffolds.iter().filter_map(|s| s(root, &name).err()).collect();
    let mut notes = Vec::new();
    let files = project::project_files(root);
    if files.len() > 1 {
        let names: Vec<String> =
            files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        problems.push(format!(
            "the folder holds {} project files ({}): one is the project, {} is used",
            files.len(),
            names.join(", "),
            names.first().cloned().unwrap_or_default()
        ));
    }
    let legacy_path = root.join(legacy::FILE);
    let legacy = gz_editor::fsx::read_opt(&legacy_path)
        .ok()
        .flatten()
        .map(|b| Legacy::parse(&gz_base::text::decode_guess(&b)));
    let (file_path, mut file) = match files.first() {
        Some(path) => {
            let text = gz_editor::fsx::read_text(path)
                .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
            let file = ProjectFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            (path.clone(), file)
        }
        None => {
            notes.push(format!("project file {name}{} written", project::EXTENSION));
            (root.join(format!("{name}{}", project::EXTENSION)), ProjectFile::fresh(&project::new_id()))
        }
    };
    if file.id().trim().is_empty() {
        file = file.with_id(&project::new_id());
    }
    if let Some(old) = &legacy
        && file.scene().is_none()
        && !old.last_scene.is_empty()
        && let Some(rel) = project::scene_in_project(&root.to_string_lossy(), &old.last_scene)
    {
        file = file.with_scene(Some(&rel));
    }
    if file.changed() {
        let text = file.to_text().map_err(|e| format!("{}: {e}", file_path.display()))?;
        gz_editor::fsx::write_atomic(&file_path, text.as_bytes())
            .map_err(|e| format!("{} could not be written: {e}", file_path.display()))?;
        file = file.written(&text);
    }
    // the old bookkeeping is read once and goes: nothing reads it again
    if legacy.is_some() {
        match std::fs::remove_file(&legacy_path) {
            Ok(()) => notes.push("project.ltx converted and deleted".to_owned()),
            Err(e) => problems.push(format!("project.ltx was converted but could not be deleted: {e}")),
        }
    }
    let state_file = machine::state_path(home, file.id());
    let mut state = gz_editor::fsx::read_opt(&state_file)
        .ok()
        .flatten()
        .map(|b| ProjectState::parse(&gz_base::text::decode_guess(&b)))
        .unwrap_or_default();
    let guide = (!state.agents_guide_written()).then(|| {
        let file_name = file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let written = guide::write_once(root, &file_name, file.id());
        state = state.with_agents_guide_written();
        written
    });
    Ok(Prepared {
        file_path,
        file,
        state,
        legacy,
        gitignore: rules::ensure_gitignore(root),
        guide,
        problems,
        notes,
    })
}

/// Opens the project at `folder`, closing the one that is open. `creating`: the folder may be
/// empty (project.create made it); otherwise it must hold a project file or `mod.ltx`.
/// `with_scene`: the project's scene comes up with it.
pub(crate) async fn open(
    ed: &Arc<Editor>,
    folder: &str,
    discard: bool,
    creating: bool,
    with_scene: bool,
) -> Result<Opened, CmdError> {
    let folder = paths::trim_trailing(folder.to_owned());
    let folder = folder.as_str();
    let probe = PathBuf::from(folder);
    let (exists, is_project) =
        ed.exec.io(move || (fsys::dir_exists(&probe), list::has_manifest(&probe))).await;
    if folder.is_empty() || !exists {
        return Err(refuse("project folder does not exist"));
    }
    if !is_project && !creating {
        return Err(refuse(format!(
            "{} is no project: it holds no project file (*.grizzly) and no mod.ltx",
            paths::wire(folder)
        )));
    }
    if ed.project_opt().is_some() {
        ed.refuse_unsaved(discard)?;
        if discard {
            ed.discard_all().await?;
        }
        close_session(ed).await?;
    }

    let hooks = ProjectHooks::of(ed);
    let scaffolds = hooks.scaffolds();
    let (root, home) = (PathBuf::from(folder), ed.paths.home.clone());
    let prepared = ed.exec.io(move || prepare(&root, &home, &scaffolds)).await.map_err(refuse)?;
    for p in &prepared.problems {
        ed.log.warn(format!("project: {p}"));
    }
    for n in &prepared.notes {
        ed.log.info(format!("Project: {n}"));
    }
    match &prepared.gitignore {
        Ok(true) => ed.log.info("Project is a git repository: .gitignore written for the editor's own files"),
        Ok(false) => {}
        Err(e) => ed.log.warn(format!("the project's .gitignore could not be written: {e}")),
    }
    match &prepared.guide {
        Some(Ok(true)) => ed.log.info("Project: AGENTS.md written"),
        Some(Err(e)) => ed.log.warn(format!("the project's AGENTS.md could not be written: {e}")),
        _ => {}
    }

    // the default game: found by itself when this home has none, the old link remembered
    let machine = Machine::of(ed);
    if let Some(old) = &prepared.legacy
        && !old.game_root.is_empty()
    {
        machine.note_seen(&old.game_root).await;
    }
    let default = machine.ensure_default_game().await;
    if let Err(why) = &default {
        ed.log.warn(format!("project: {why}"));
    }
    let mut state = prepared.state.with_root(folder);
    if let Some(old) = &prepared.legacy {
        let default_root = default.clone().unwrap_or_default();
        if !old.game_root.is_empty() && !paths::same_path(&old.game_root, &default_root) {
            state = state.with_game_root(&old.game_root);
        }
        if !old.export_target.is_empty() && state.export_target().is_empty() {
            state = state.with_export_target(&old.export_target);
        }
    }

    let session = Arc::new(ProjectSession::new(PathBuf::from(folder)));
    let file = prepared.file.clone();
    let id = file.id().to_owned();
    let loading = state.loading_scene();
    let project = session.services.insert(Project::new(
        ed,
        folder,
        prepared.file_path.clone(),
        file.clone(),
        ProjectState::default(),
    ));
    if let Err(e) = project.edit_state(|_| state.clone()).await {
        ed.log.warn(e.message);
    }
    project.check_own_game().await;

    // the scene to reopen: inside this folder only - a copied project must never open, and then
    // save into, the scene of the one it was copied from
    if let Some(stored) = file.scene().map(str::to_owned) {
        let root = folder.to_owned();
        let found = ed
            .exec
            .io(move || {
                let rel = project::scene_in_project(&root, &stored)?;
                let exists = std::fs::symlink_metadata(Path::new(&root).join(&rel)).is_ok();
                Some((rel, exists))
            })
            .await;
        match found {
            None => {
                // a project names no place outside it: a copy must never open, and then save into,
                // the scene of the one it was copied from
                ed.log.warn(
                    "project: the scene to open lies outside the project: it is not opened and is forgotten",
                );
                if let Err(e) = project.edit_file(ProjectFile::without_scene).await {
                    ed.log.warn(e.message);
                }
            }
            Some((rel, true)) => {
                let rel_path = RelPath::new(&rel).ok();
                let crashed =
                    !loading.is_empty() && rel_path.as_ref().is_some_and(|r| r.to_string() == loading);
                if crashed {
                    ed.log.warn(format!(
                        "project: the last start did not finish loading {loading}, so it is not opened by itself this time"
                    ));
                    let _ = project.edit_state(|s| s.with_loading_scene("")).await;
                } else if with_scene {
                    project.set_pending_scene(rel_path);
                }
            }
            // a stale path is skipped silently
            Some((_, false)) => {}
        }
    }

    ed.set_project(Some(session.clone()));
    remember_opened(ed, folder, &id).await;
    ed.log.reset_counts();
    ed.log.info(format!("Project opened: {folder}"));
    hooks.run_open(ed, &session);
    let file_name =
        prepared.file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Opened { root: folder.to_owned(), id, file: file_name })
}

/// The project list after `folder` was open: it goes first, in place of an entry of the same
/// project at another place. A list that cannot be written is logged; the project is open.
async fn remember_opened(ed: &Arc<Editor>, folder: &str, id: &str) {
    let projects = Projects::of(ed);
    let create_parent = Projects::create_parent(&projects.list(ed).await);
    if let Err(e) = projects.edit(|l| l.opened(folder, id, &create_parent)).await {
        ed.log.warn(e.message);
    }
}

/// Closes the open project when the editor quits: nothing is asked - the exit prompt settled the
/// tabs, and graphs nobody settled are written by their own close hook.
pub async fn close_project(ed: &Arc<Editor>) {
    if let Err(e) = close_session(ed).await {
        ed.log.warn(format!("the project did not close cleanly: {}", e.message));
    }
}

/// Closes the open project: its hooks, the scene to open next time, the list, then the session
/// goes. The caller settled unsaved work.
pub(crate) async fn close_session(ed: &Arc<Editor>) -> Result<(), CmdError> {
    let Some(session) = ed.project_opt() else { return Ok(()) };
    let hooks = ProjectHooks::of(ed);
    hooks.run_before_close(ed, &session).await;
    hooks.run_close(ed, &session);
    if let Some(project) = session.services.get::<Project>() {
        let root = project.root().to_owned();
        let scene = project.scene_to_remember();
        let check_root = root.clone();
        // a scene from outside the project is not reopened
        let rel = ed.exec.io(move || scene.and_then(|s| project::scene_in_project(&check_root, &s))).await;
        if let Some(rel) = &rel {
            let wire = rel.replace('\\', "/");
            if let Err(e) = project.edit_file(|f| f.with_scene(Some(&wire))).await {
                ed.log.warn(e.message);
            }
            if let Err(e) = project.edit_state(|s| s.with_recent_scene(&wire).with_loading_scene("")).await {
                ed.log.warn(e.message);
            }
        }
        remember_opened(ed, &root, project.id()).await;
    }
    ed.set_project(None);
    Ok(())
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CloseParams {
    /// Save the unsaved work first.
    #[serde(default)]
    save: bool,
    /// Drop the unsaved work.
    #[serde(default)]
    discard: bool,
}

async fn project_close(ed: Arc<Editor>, p: CloseParams, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(refuse("no project is open: the Start page is up"));
    }
    if p.save && p.discard {
        return Err(CmdError::invalid("save and discard exclude each other"));
    }
    let dirty = ed.dirty_docs();
    let dirty_json = serde_json::to_value(&dirty).unwrap_or(Value::Null);
    if !dirty.is_empty() {
        if p.save {
            // every area saves its own documents; the first that cannot names itself
            ed.save_all().await?;
            let left = ed.dirty_docs();
            if let Some(d) = left.first() {
                return Err(refuse_with(
                    format!("{}: it is still unsaved", d.name),
                    Map::from_iter([(
                        "dirty".to_owned(),
                        serde_json::to_value(&left).unwrap_or(Value::Null),
                    )]),
                ));
            }
        } else {
            if !lets_go(p.discard, origin) {
                return Err(CmdError::unsaved(dirty_json.clone())
                    .with_data(json!({ "dirty": dirty_json, "note": "pass save=true or discard=true" })));
            }
            ed.discard_all().await?;
        }
    }
    close_session(&ed).await?;
    let mut out = Map::new();
    out.insert("start_page".into(), json!(true));
    if !dirty.is_empty() {
        out.insert(if p.save { "saved" } else { "discarded" }.into(), dirty_json);
    }
    Ok(Value::Object(out))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateParams {
    /// The project's name: its folder's and its project file's. Latin letters, digits, _ and -.
    name: String,
    /// The folder the project's folder is made in; left out: the Start page form's location.
    #[serde(default)]
    parent: Option<String>,
}

async fn project_create(ed: Arc<Editor>, p: CreateParams, _origin: Origin) -> Reply {
    let name = p.name.trim().to_owned();
    let projects = Projects::of(&ed);
    let list = projects.list(&ed).await;
    let parent = paths::trim_trailing(
        p.parent.map(|s| paths::to_backslash(&s)).unwrap_or_else(|| Projects::create_parent(&list)),
    );
    if ed.project_opt().is_some() {
        return Err(refuse("a project is open: project.close first, then create"));
    }
    if parent.len() + name.len() + 2 > MAX_PATH {
        return Err(refuse("the path is too long"));
    }
    if !valid_name(&name) {
        return Err(refuse("The name may hold Latin letters, digits, _ and - only."));
    }
    // a relative location would land in the editor's working folder
    if !paths::is_absolute(&parent) {
        return Err(refuse("The location must be a full path."));
    }
    let folder = paths::join(&parent, &name);
    let probe = PathBuf::from(&folder);
    let existed = ed.exec.io(move || fsys::dir_exists(&probe) && list::has_manifest(&probe)).await;
    if !existed {
        let target = PathBuf::from(&folder);
        let made = ed
            .exec
            .io(move || {
                std::fs::create_dir_all(&target)
                    .map_err(|e| fsys::win32(&e))
                    .map(|()| fsys::dir_exists(&target))
            })
            .await;
        match made {
            Ok(true) => {}
            Ok(false) => {
                return Err(refuse(fsys::win32_text(&format!("The folder {folder} cannot be created"), 0)));
            }
            Err(code) => {
                return Err(refuse(fsys::win32_text(
                    &format!("The folder {folder} cannot be created"),
                    code,
                )));
            }
        }
    }
    // a project there already is simply opened; the skeleton and the project file are the open's
    let opened = open(&ed, &folder, false, true, true).await?;
    Ok(
        json!({ "path": paths::wire(&folder), "name": name, "existed": existed, "opened": true, "id": opened.id, "file": opened.file }),
    )
}

// ---- the Start page's card menu -----------------------------------------------------------------

/// The fields a deletion answers with. `counts` names the object the numbers go into.
fn delete_fields(d: &DeleteInfo, counts: Option<&str>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("path".into(), json!(paths::wire(&d.path)));
    m.insert("name".into(), json!(paths::wire(&d.name)));
    m.insert("refused".into(), json!(d.refused.is_some()));
    if d.missing {
        m.insert("missing".into(), json!(true));
    }
    let numbers = json!({ "files": d.files, "folders": d.folders, "links": d.links, "bytes": d.bytes });
    match (counts, numbers) {
        (Some(key), numbers) => {
            m.insert(key.to_owned(), numbers);
        }
        (None, Value::Object(flat)) => m.extend(flat),
        (None, _) => {}
    }
    m.insert("git".into(), json!(d.git));
    m.insert("releases".into(), json!(d.releases));
    m.insert("release_files".into(), json!(d.release_files));
    m.insert("release_bytes".into(), json!(d.release_bytes));
    m.insert("nested_total".into(), json!(d.nested_total));
    m.insert("failed_total".into(), json!(d.failed_total));
    m.insert("nested".into(), d.nested.iter().map(|n| json!(paths::wire(n))).collect());
    m.insert(
        "failed".into(),
        d.failed
            .iter()
            .map(|f| json!({ "path": paths::wire(&f.path), "reason": paths::wire(&f.reason) }))
            .collect(),
    );
    m
}

/// What the delete checks compare with, gathered on the IO pool.
pub(crate) async fn delete_context(ed: &Arc<Editor>) -> DeleteContext {
    let listed = Projects::of(ed).list(ed).await.entries();
    let open = Project::of(ed).ok().map(|(_, p)| (p.root().to_owned(), p.game_root()));
    let (exe, root, home) = (ed.paths.exe_dir.clone(), ed.paths.root.clone(), ed.paths.home.clone());
    ed.exec
        .io(move || {
            let guards =
                delete::system_guards(&exe, &root, &home, open.as_ref().map(|o| o.1.as_str()), &listed);
            DeleteContext { listed, open: open.map(|o| o.0), guards }
        })
        .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListedPath {
    /// A project folder of the list (project.list).
    path: String,
}

async fn project_delete(ed: Arc<Editor>, p: ListedPath, _origin: Origin) -> Reply {
    let path = paths::to_backslash(&p.path);
    // the caller decided: no confirmation on this path, the same checks as the window's
    let cx = delete_context(&ed).await;
    let outcome = ed.exec.io(move || delete::delete_project(&cx, &path)).await;
    match outcome {
        Ok(mut info) => {
            let gone = info.path.clone();
            Projects::of(&ed).edit(|l| l.without(&gone)).await?;
            info.removed_from_list = true;
            if info.missing {
                ed.log
                    .info(format!("Project list: {} removed, the folder does not exist any more", info.path));
            } else {
                ed.log.info(format!(
                    "Project deleted: {} ({} files, {} folders, {} links)",
                    info.path, info.files, info.folders, info.links
                ));
            }
            let mut out = Map::new();
            if info.missing {
                out.insert(
                    "note".into(),
                    json!("the folder does not exist any more - removed from the list only"),
                );
            }
            out.extend(delete_fields(&info, Some("deleted")));
            out.insert("removed_from_list".into(), json!(true));
            Ok(Value::Object(out))
        }
        Err(failed) => {
            if let Some(first) = failed.info.failed.first() {
                ed.log.error(format!(
                    "Project {} not deleted completely: {} entries stayed behind, the first: {} ({})",
                    failed.info.path, failed.info.failed_total, first.path, first.reason
                ));
            }
            let mut fields = delete_fields(&failed.info, Some("deleted"));
            fields.insert("removed_from_list".into(), json!(false));
            Err(refuse_with(paths::wire(&failed.message), fields))
        }
    }
}

async fn project_forget(ed: Arc<Editor>, p: ListedPath, _origin: Origin) -> Reply {
    let path = paths::to_backslash(&p.path);
    let projects = Projects::of(&ed);
    let list = projects.list(&ed).await;
    let Some(listed) = list.find_listed(&path) else { return Err(refuse("It is not in the project list.")) };
    let probe = listed.clone();
    if ed.exec.io(move || list::entry_state(&probe)).await == EntryState::Project {
        return Err(refuse("The folder still holds a project, so it stays in the list."));
    }
    projects.edit(|l| l.without(&listed)).await?;
    Ok(json!({ "path": paths::wire(&path), "removed_from_list": true }))
}

fn launch_explorer(folder: &str) -> Result<(), String> {
    let folder = paths::trim_trailing(folder.to_owned());
    let mut cmd = std::process::Command::new("explorer.exe");
    // quoted, a path with spaces being the normal case; a drive root needs no quotes and would
    // read its own trailing separator as an escaped quote
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if folder.ends_with('\\') {
            cmd.raw_arg(&folder);
        } else {
            cmd.raw_arg(format!("\"{folder}\""));
        }
    }
    #[cfg(not(windows))]
    cmd.arg(&folder);
    cmd.spawn().map(|_| ()).map_err(|e| format!("Windows did not start Explorer ({e})."))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RevealParams {
    /// A project folder of the list (project.list).
    path: String,
    /// Answer the folder and open nothing (a test leaves no window behind).
    #[serde(default)]
    dry_run: bool,
}

async fn project_reveal(ed: Arc<Editor>, p: RevealParams, _origin: Origin) -> Reply {
    let path = paths::to_backslash(&p.path);
    let list = Projects::of(&ed).list(&ed).await;
    let Some(folder) = list.find_listed(&path) else {
        return Err(refuse("It is not in the project list - only a listed project is shown from here."));
    };
    let probe = PathBuf::from(&folder);
    if !ed.exec.io(move || fsys::dir_exists(&probe)).await {
        return Err(refuse("The folder does not exist any more."));
    }
    if !p.dry_run {
        launch_explorer(&folder).map_err(refuse)?;
    }
    Ok(json!({ "path": paths::wire(&folder), "opened": !p.dry_run, "dry_run": p.dry_run }))
}

// ---- what the open project is -------------------------------------------------------------------

async fn project_info(ed: Arc<Editor>, _p: NoArgs, _origin: Origin) -> Reply {
    let (_, project) = Project::of(&ed)?;
    let (root, source) = project.game_source();
    let probe = root.clone();
    let reason =
        ed.exec.io(move || gz_vfs::looks_like_game(&probe).err().unwrap_or_default().to_owned()).await;
    let file = project.file();
    let file_root = PathBuf::from(project.root());
    let problems = {
        let f = file.clone();
        ed.exec.io(move || f.problems(&file_root)).await
    };
    let machine = Machine::of(&ed);
    Ok(json!({
        "path": paths::wire(project.root()),
        "name": project.name(),
        "file": project.file_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "id": project.id(),
        "scene": file.scene().unwrap_or_default(),
        "library": file.library(),
        "game": {
            "root": paths::wire(&root),
            "source": source.key(),
            "valid": reason.is_empty() && !root.is_empty(),
            "reason": reason,
            "default": paths::wire(&machine.default_game()),
        },
        "library_root": paths::wire(&machine.library_root().display().to_string()),
        "problems": problems,
    }))
}

async fn project_guide(ed: Arc<Editor>, _p: NoArgs, _origin: Origin) -> Reply {
    let (_, project) = Project::of(&ed)?;
    let root = PathBuf::from(project.root());
    let name = project.file_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let id = project.id().to_owned();
    ed.exec
        .io(move || guide::rewrite(&root, &name, &id))
        .await
        .map_err(|e| CmdError::io(format!("AGENTS.md could not be written: {e}")))?;
    project.edit_state(ProjectState::with_agents_guide_written).await?;
    project.tree.mark_fs(Path::new(project.root()));
    Ok(json!({ "path": guide::FILE }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CollectParams {
    /// Only say how many files and bytes it would copy; nothing is written.
    #[serde(default)]
    dry_run: bool,
    /// Wait for the copy to end (the default); false answers the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

async fn project_collect(ed: Arc<Editor>, p: CollectParams, _origin: Origin) -> Reply {
    let (answer, job) = library::collect(&ed, p.dry_run).await?;
    let mut out = serde_json::to_value(&answer).unwrap_or(Value::Null);
    out["dry_run"] = json!(p.dry_run);
    let Some(job) = job else { return Ok(out) };
    if p.wait == Some(false) {
        out["job"] = json!(job.0);
        return Ok(out);
    }
    let info = ed.jobs.wait(job).await;
    match info {
        Some(info) if info.error.is_none() => {
            out["job"] = json!(job.0);
            if let Some(result) = info.result {
                out["copied"] = result.get("copied").cloned().unwrap_or(Value::Null);
            }
            Ok(out)
        }
        Some(info) => Err(CmdError::new("failed", info.error.unwrap_or_default())),
        None => Err(CmdError::internal("the collect job is gone")),
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LibraryParams {
    /// The folder that holds the library's rawdata and gamedata; empty or left out: the
    /// editor's own, beside it.
    #[serde(default)]
    path: Option<String>,
}

async fn project_set_library(ed: Arc<Editor>, p: LibraryParams, _origin: Origin) -> Reply {
    let path = crate::games::clean_root(&p.path.unwrap_or_default());
    if !path.is_empty() {
        let probe = PathBuf::from(&path);
        let fits =
            ed.exec.io(move || probe.join("rawdata").is_dir() || probe.join("gamedata").is_dir()).await;
        if !fits {
            return Err(refuse(format!(
                "{} holds neither rawdata nor gamedata: it is no library",
                paths::wire(&path)
            )));
        }
        // the library is written (descriptions, game textures and sounds, light animations): a
        // game's folder is no library, whether this editor knows the game or the folder holds
        // what only a game has (a folder laid out like the editor's own is a library)
        let guard = crate::games::library_guard(&ed);
        let probe = PathBuf::from(&path);
        let a_game =
            ed.exec.io(move || guard.check(&probe).is_err() || crate::games::holds_a_game(&probe)).await;
        if a_game {
            return Err(refuse(format!(
                "{} is a game's folder: the library is the editor's own, and a game is never written",
                paths::wire(&path)
            )));
        }
    }
    let machine = Machine::of(&ed);
    machine.edit(|m| m.with_library_root(&path)).await?;
    Ok(json!({ "path": paths::wire(&machine.library_root().display().to_string()), "own": path.is_empty() }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssociateParams {
    /// Whether the editor keeps .grizzly files opening with it (Preferences > General); left
    /// out: as it is.
    #[serde(default)]
    on: Option<bool>,
    /// Write the association for this editor now (Register Now).
    #[serde(default)]
    now: bool,
}

async fn project_associate(ed: Arc<Editor>, p: AssociateParams, _origin: Origin) -> Reply {
    let machine = Machine::of(&ed);
    if let Some(on) = p.on {
        machine.edit(|m| m.with_associate(on)).await?;
    }
    let on = machine.load().await.map(|m| m.associate()).unwrap_or(true);
    let program = crate::association::this_program()
        .ok_or_else(|| CmdError::internal("the editor's own program is not known"))?;
    let now = p.now;
    let state = ed
        .exec
        .io(move || -> Result<crate::association::State, String> {
            if now {
                crate::association::register(crate::association::CLASSES, &program)
                    .map_err(|e| format!("The association could not be written: {e}"))?;
            }
            Ok(crate::association::state(crate::association::CLASSES, &program))
        })
        .await
        .map_err(CmdError::io)?;
    Ok(
        json!({ "on": on, "program": state.program.map(|p| paths::wire(&p)), "ours": state.ours, "exists": state.exists }),
    )
}

// ---- the project's files ------------------------------------------------------------------------

/// The open project for a file command.
fn file_project(ed: &Editor) -> Result<Arc<Project>, CmdError> {
    Project::of(ed).map(|(_, p)| p).map_err(|_| CmdError::no_project())
}

/// One path or several.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum Paths {
    One(String),
    Many(Vec<String>),
}

impl Paths {
    fn list(&self) -> Vec<String> {
        match self {
            Paths::One(p) => vec![p.clone()],
            Paths::Many(v) => v.clone(),
        }
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
    }
}

/// `{ok, files, folders, skipped, failed, errors, skips}`; a report with failures is `ok:false`.
fn report_reply(rep: &Report) -> Reply {
    let fields = rep.fields();
    if rep.ok() {
        return Ok(Value::Object(fields));
    }
    // `failed` and `errors` say what went wrong
    Err(CmdError::report(Value::Object(fields)))
}

#[derive(Clone, Copy)]
enum Transfer {
    Copy,
    Move,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TransferParams {
    /// The files or folders of the project (project-relative, or absolute inside it).
    path: Paths,
    /// The folder of the project they go into.
    to: String,
    /// Folders with everything in them (the default).
    #[serde(default)]
    recursive: Option<bool>,
    /// Replace files that are there already: they are lost.
    #[serde(default)]
    overwrite: bool,
}

async fn transfer(ed: Arc<Editor>, p: TransferParams, op: Transfer) -> Reply {
    let project = file_project(&ed)?;
    let (entries, dst) = (p.path.list(), p.to.clone());
    if entries.is_empty() {
        return Err(CmdError::invalid("path names nothing"));
    }
    let recursive = p.recursive.unwrap_or(true);
    let overwrite = p.overwrite;
    let pr = project.clone();
    let (rep, touched) = ed
        .exec
        .io(move || {
            let game = || pr.game_files().ok();
            let mut ops = FileOps::new(pr.root(), &game, pr.write_guard(None));
            let mut rep = Report::default();
            for entry in entries {
                match op {
                    Transfer::Copy => ops.copy(&entry, &dst, recursive, overwrite, &mut rep),
                    Transfer::Move => ops.move_entry(&entry, &dst, recursive, overwrite, &mut rep),
                }
            }
            (rep, ops.touched)
        })
        .await;
    for dir in touched {
        project.tree.mark_fs(Path::new(&dir));
    }
    report_reply(&rep)
}

async fn file_copy(ed: Arc<Editor>, p: TransferParams, _origin: Origin) -> Reply {
    transfer(ed, p, Transfer::Copy).await
}

async fn file_move(ed: Arc<Editor>, p: TransferParams, _origin: Origin) -> Reply {
    transfer(ed, p, Transfer::Move).await
}

fn no_game() -> Option<Arc<GameFiles>> {
    None
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    /// The files or folders of the project.
    path: Paths,
    /// A folder goes with everything in it.
    #[serde(default)]
    recursive: bool,
}

async fn file_delete(ed: Arc<Editor>, p: DeleteParams, _origin: Origin) -> Reply {
    let project = file_project(&ed)?;
    let entries = p.path.list();
    let recursive = p.recursive;
    let pr = project.clone();
    let (rep, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(pr.root(), &no_game, pr.write_guard(None));
            let mut rep = Report::default();
            for entry in entries {
                ops.delete(&entry, recursive, &mut rep);
            }
            (rep, ops.touched)
        })
        .await;
    for dir in touched {
        project.tree.mark_fs(Path::new(&dir));
    }
    report_reply(&rep)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OnePath {
    /// A folder of the project.
    path: String,
}

async fn file_mkdir(ed: Arc<Editor>, p: OnePath, _origin: Origin) -> Reply {
    let project = file_project(&ed)?;
    let pr = project.clone();
    let (made, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(pr.root(), &no_game, pr.write_guard(None));
            (ops.mkdir(&p.path), ops.touched)
        })
        .await;
    for dir in touched {
        project.tree.mark_fs(Path::new(&dir));
    }
    let full = made.map_err(refuse)?;
    Ok(json!({ "path": paths::wire(&rel_or_full(project.root(), &full)) }))
}

/// A path of the project, project-relative; anything else as it is.
fn rel_or_full(root: &str, path: &str) -> String {
    project::scene_in_project(root, path).unwrap_or_else(|| path.to_owned())
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameParams {
    /// The file or folder of the project.
    path: String,
    /// The new name, without a folder. The editor's own assets keep their extension.
    name: String,
}

async fn file_rename(ed: Arc<Editor>, p: RenameParams, _origin: Origin) -> Reply {
    let project = file_project(&ed)?;
    let pr = project.clone();
    let (renamed, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(pr.root(), &no_game, pr.write_guard(None));
            (ops.rename(&p.path, &p.name), ops.touched)
        })
        .await;
    for dir in touched {
        project.tree.mark_fs(Path::new(&dir));
    }
    let (from, to) = renamed.map_err(refuse)?;
    if from != to {
        // open documents follow the file, unsaved edits included
        ProjectHooks::of(&ed).run_renamed(&ed, Path::new(&from), Path::new(&to));
    }
    Ok(json!({ "path": paths::wire(&rel_or_full(project.root(), &to)) }))
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CreateKind {
    Quest,
    Behaviour,
    Character,
    Pda,
    Item,
    Model,
    Texture,
    Sound,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileCreateParams {
    /// What to make.
    kind: CreateKind,
    /// The folder of the project it goes into; left out: the folder the Content Browser shows.
    #[serde(default)]
    path: Option<String>,
}

async fn file_create(ed: Arc<Editor>, p: FileCreateParams, _origin: Origin) -> Reply {
    let project = file_project(&ed)?;
    let kind = match p.kind {
        CreateKind::Quest => TemplateKind::Quest,
        CreateKind::Behaviour => TemplateKind::Behaviour,
        CreateKind::Character => TemplateKind::Character,
        CreateKind::Pda => TemplateKind::Pda,
        CreateKind::Item => TemplateKind::Item,
        CreateKind::Model => TemplateKind::Model,
        CreateKind::Texture => TemplateKind::Texture,
        CreateKind::Sound => TemplateKind::Sound,
    };
    let folder = match p.path {
        Some(f) => RelPath::new(&f).map_err(|e| refuse(e.to_string()))?,
        None => {
            let shown = ProjectHooks::of(&ed).content_folder(&ed).map_err(refuse)?;
            if shown.is_empty() {
                RelPath::root()
            } else {
                RelPath::new(&shown).map_err(|e| refuse(e.to_string()))?
            }
        }
    };
    let Some(template) = AssetTemplates::of(&ed).get(kind) else {
        return Err(refuse(format!(
            "Cannot create the {}: this build has no {} template",
            kind.noun(),
            kind.key()
        )));
    };
    if let Some(why) = folder_problem(&folder) {
        return Err(refuse(format!("Cannot create the {}: {why}", kind.noun())));
    }
    // the names the kind's files have anywhere in the project, as the tree holds them now: the new
    // asset's id comes from its name, and two files of one name in two folders would share it
    project.tree.refresh().await;
    let extension = kind.leaf(1).rsplit('.').next().unwrap_or_default().to_owned();
    let taken: rustc_hash::FxHashSet<String> = project
        .tree
        .snapshot()
        .files_with_extension(&extension)
        .iter()
        .map(|f| f.file_name().to_lowercase())
        .collect();
    let root = PathBuf::from(project.root());
    let rel_folder = folder.clone();
    let pr = project.clone();
    let made = ed
        .exec
        .io(move || -> Result<RelPath, String> {
            let refused = |why: String| format!("Cannot create the {}: {why}", kind.noun());
            let dir = rel_folder.to_fs(&root);
            if !fsys::dir_exists(&dir) {
                // a file in the way is named, not what the system calls the failure
                let mut at = root.clone();
                for seg in rel_folder.segments() {
                    at.push(seg);
                    if fsys::file_exists(&at) {
                        return Err(refused(format!(
                            "{seg} is a file, not a folder ({} cannot be made)",
                            rel_folder.to_backslash()
                        )));
                    }
                }
                // the folder it was asked to put the asset into is made, the way file.mkdir makes one
                let mut ops = FileOps::new(pr.root(), &no_game, pr.write_guard(None));
                ops.mkdir(&dir.to_string_lossy())
                    .map_err(|e| refused(format!("the folder {}: {e}", rel_folder.to_backslash())))?;
            }
            let found = (1..=999u32).map(|n| (n, kind.leaf(n))).find(|(_, leaf)| {
                !taken.contains(&leaf.to_lowercase()) && std::fs::symlink_metadata(dir.join(leaf)).is_err()
            });
            let Some((n, leaf)) = found else { return Err(kind.too_many().to_owned()) };
            let stem = leaf.rfind('.').and_then(|d| leaf.get(..d)).unwrap_or(&leaf).to_owned();
            let bytes = template(&TemplateCx { stem: &stem, n }).map_err(|e| refused(e.to_string()))?;
            // the file passes the guard where it really lands: a folder that is a link may lead
            // into the linked game
            let file = dir.join(&leaf);
            pr.write_guard(None).check(&file).map_err(|e| refused(e.to_string()))?;
            gz_editor::fsx::write_atomic(&file, &bytes).map_err(|e| refused(e.to_string()))?;
            rel_folder.join(&leaf).map_err(|e| e.to_string())
        })
        .await
        .map_err(refuse)?;
    // a folder made on the way shows in its parent
    project.tree.mark_fs(Path::new(project.root()));
    project.tree.mark(folder);
    Ok(json!({ "kind": kind.key(), "path": made.to_string() }))
}

/// Why an asset is not made in `folder`, or `None`: a folder the project never lists (tooling at
/// any depth, the editor's own folders at the root), or a name Windows does not take as it is.
fn folder_problem(folder: &RelPath) -> Option<String> {
    for (depth, seg) in folder.segments().enumerate() {
        if rules::is_tooling_entry(seg) {
            return Some(format!(
                "{seg} is a tooling folder: the project lists nothing in it and no build ships it"
            ));
        }
        if depth == 0 && rules::is_editor_only_entry(seg) {
            return Some(format!("{seg} is the editor's own folder: nothing in it is the project's content"));
        }
        if let Some(why) = gz_base::file_name_problem(seg) {
            return Some(format!("the folder name '{seg}' {why}"));
        }
    }
    None
}

/// Where `file.list` and `file.import` look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Source {
    /// The open project's files.
    #[default]
    Project,
    /// The editor's library (Preferences > Library).
    Library,
    /// The game's files, archives included.
    Game,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListParams {
    /// Where to list.
    #[serde(default)]
    source: Source,
    /// A folder to list below (project-relative, or a game path); left out: everything.
    #[serde(default)]
    path: Option<String>,
    /// The project: an asset kind (quest, behaviour, character, pda) or an extension (dds). The
    /// library: objects, levels, groups, visuals, textures, raw_textures, particles, sounds,
    /// entities, light_anims (objects when left out). The game: an extension.
    #[serde(default)]
    kind: Option<String>,
    /// Only names that hold this text, any case.
    #[serde(default)]
    query: Option<String>,
    /// At most this many (100 when left out).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many first.
    #[serde(default)]
    offset: Option<usize>,
}

/// The library categories by the words `file.list` takes.
fn library_category(kind: &str) -> Option<&'static str> {
    let k = kind.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    let k = k.trim_start_matches('.');
    let word = match k {
        "" | "objects" | "object" => "Objects",
        "levels" | "level" => "Levels",
        "groups" | "group" => "Groups",
        "visuals" | "visual" => "Visuals",
        "textures" | "texture" => "Textures",
        "raw_textures" | "textures_(raw)" | "raw_texture" => "Textures (raw)",
        "particles" | "particle" => "Particles",
        "sounds" | "sound" => "Sounds",
        "entities" | "entity" => "Entities",
        "light_anims" | "light_anim" => "Light Anims",
        _ => return None,
    };
    CATEGORIES.iter().copied().find(|c| *c == word)
}

/// `Levels`: the project's scenes (`rawdata\levels`, names without `.level`) and the library's
/// base scenes the project does not hold.
async fn level_names(ed: &Arc<Editor>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok((_, project)) = Project::of(ed) {
        project.tree.refresh().await;
        let tree = project.tree.snapshot();
        if let Ok(levels) = RelPath::new("rawdata/levels") {
            for (path, _) in tree.files_under(&levels) {
                if !path.has_extension("level") {
                    continue;
                }
                if let Some(rel) = path.with_extension("").strip_prefix(&levels) {
                    names.push(rel.to_backslash());
                }
            }
        }
    }
    let root = crate::games::library_root(ed);
    let base = ed.exec.io(move || project::base_scenes(&root)).await;
    for b in base {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&b)) {
            names.push(b);
        }
    }
    names
}

/// The `limit` / `offset` cut of a list: `(shown, total)`.
fn page<T>(items: Vec<T>, offset: usize, limit: usize) -> (Vec<T>, usize) {
    let total = items.len();
    (items.into_iter().skip(offset).take(limit).collect(), total)
}

async fn file_list(ed: Arc<Editor>, p: ListParams, _origin: Origin) -> Reply {
    let query = p.query.unwrap_or_default().to_lowercase();
    let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(100).max(1));
    let kind = p.kind.unwrap_or_default();
    let matches = |s: &str| query.is_empty() || s.to_lowercase().contains(&query);
    match p.source {
        Source::Project => {
            let project = file_project(&ed)?;
            project.tree.refresh().await;
            let tree = project.tree.snapshot();
            let under = match p.path.as_deref().filter(|s| !s.is_empty()) {
                Some(s) => RelPath::new(s).map_err(|e| CmdError::invalid(e.to_string()))?,
                None => RelPath::root(),
            };
            let template = TemplateKind::parse(&kind);
            let ext = kind.trim().trim_start_matches('.').to_ascii_lowercase();
            let mut items = Vec::new();
            for (path, entry) in tree.files_under(&under) {
                let crate::tree::EntryKind::File { size, asset, .. } = entry.kind else { continue };
                let wire = path.to_string();
                let top = wire.split('/').next().unwrap_or_default();
                if rules::is_editor_only_entry(top) {
                    continue;
                }
                let kind_ok = match template {
                    // the kind's extension: a media asset is no kind the tree tells apart
                    Some(t) => t.leaf(1).rsplit_once('.').is_some_and(|(_, e)| path.has_extension(e)),
                    None => ext.is_empty() || path.has_extension(&ext),
                };
                if kind_ok && matches(&wire) {
                    items.push(json!({ "path": wire, "size": size, "kind": asset.map(|a| a.name()) }));
                }
            }
            let (files, total) = page(items, offset, limit);
            Ok(json!({ "source": "project", "total": total, "files": files }))
        }
        Source::Library => {
            let Some(category) = library_category(&kind) else {
                return Err(refuse_with(
                    "unknown kind for the library",
                    Map::from_iter([("kinds".to_owned(), json!(CATEGORIES))]),
                ));
            };
            let folder = match category {
                "Visuals" => Some(("gamedata", "meshes", "ogf")),
                "Textures" => Some(("gamedata", "textures", "dds")),
                "Textures (raw)" => Some(("rawdata", "textures", "tga")),
                "Sounds" => Some(("gamedata", "sounds", "ogg")),
                _ => None,
            };
            let items = if category == "Levels" {
                level_names(&ed).await
            } else if let Some((top, dir, ext)) = folder {
                let dir = crate::games::library_root(&ed).join(top).join(dir);
                ed.exec.io(move || crate::hooks::editor_file_list(&dir, ext)).await
            } else {
                match AssetLists::of(&ed).get(category) {
                    Some(Lister::Io(provider)) => {
                        let editor = ed.clone();
                        ed.exec.io(move || provider(&editor)).await
                    }
                    Some(Lister::Async(provider)) => provider(ed.clone()).await,
                    None => {
                        return Err(refuse(
                            "nothing lists this kind: the area that reads it is not installed",
                        ));
                    }
                }
            };
            let under = p.path.unwrap_or_default().replace('/', "\\").to_lowercase();
            let items: Vec<Value> = items
                .into_iter()
                .filter(|n| under.is_empty() || n.to_lowercase().starts_with(&under))
                .filter(|n| matches(n))
                .map(|n| json!({ "path": n }))
                .collect();
            let (files, total) = page(items, offset, limit);
            Ok(json!({ "source": "library", "kind": category, "total": total, "files": files }))
        }
        Source::Game => {
            let (_, index) = mounted(&ed).await?;
            let ext = kind.trim().trim_start_matches('.').to_lowercase();
            let under = p.path.unwrap_or_default().replace('\\', "/").to_lowercase();
            let mut items = Vec::new();
            for it in index.iter() {
                if !under.is_empty() && !it.path.starts_with(under.trim_end_matches('/')) {
                    continue;
                }
                if !ext.is_empty() {
                    let Some(e) = it.path.rfind('.').and_then(|d| it.path.get(d + 1..)) else { continue };
                    if !e.eq_ignore_ascii_case(&ext) {
                        continue;
                    }
                }
                if matches(it.path) {
                    items.push(
                        json!({ "path": paths::wire(it.path), "size": it.size, "archive": it.source_name() }),
                    );
                }
            }
            let (files, total) = page(items, offset, limit);
            Ok(json!({ "source": "game", "total": total, "files": files }))
        }
    }
}

/// The game's index of the open project, mounted on first need.
async fn mounted(ed: &Arc<Editor>) -> Result<(Arc<Project>, Arc<GameFiles>), CmdError> {
    let project = file_project(ed)?;
    let p = project.clone();
    let files = ed.exec.io(move || p.game_files()).await.map_err(refuse)?;
    Ok((project, files))
}

/// Copies a file of the game to `<project>\<to>`. `Ok((target, existed))`, or the target,
/// whether it existed and why not.
fn copy_from_game(
    files: &GameFiles,
    project_root: &str,
    rel: &str,
    to: &str,
    overwrite: bool,
    guard: &gz_editor::fsx::WriteGuard,
) -> Result<(String, bool), (String, bool, String)> {
    let Some(item) = files.get(rel) else {
        return Err((String::new(), false, "the game has no such file".into()));
    };
    let target = paths::join(project_root, &paths::to_backslash(to));
    if let Err(e) = guard.check(Path::new(&target)) {
        return Err((target, false, e.to_string()));
    }
    let existed = fsys::file_exists(Path::new(&target));
    if existed && !overwrite {
        return Err((target, true, "it is in the project already (overwrite=true replaces it)".into()));
    }
    let Ok(data) = files.read_ref(item) else {
        return Err((target, existed, "the game's file cannot be read".into()));
    };
    if let Some(parent) = Path::new(&target).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    fsys::clear_readonly(Path::new(&target));
    if let Err(e) = gz_editor::fsx::write_atomic(Path::new(&target), &data) {
        return Err((target, existed, format!("it cannot be written: {e}")));
    }
    Ok((target, existed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ImportFrom {
    /// The editor's library: a path below it (`rawdata/objects/props/box.object`).
    Library,
    /// The game: a path of its files (`textures/ui/ui_icon.dds`).
    Game,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    /// Where the file comes from.
    from: ImportFrom,
    /// The file there.
    path: String,
    /// Where it goes in the project; left out: the same path (a game file under `gamedata`).
    #[serde(default)]
    to: Option<String>,
    /// Replace a file the project has there already: it is lost.
    #[serde(default)]
    overwrite: bool,
}

async fn file_import(ed: Arc<Editor>, p: ImportParams, _origin: Origin) -> Reply {
    let project = file_project(&ed)?;
    match p.from {
        ImportFrom::Game => {
            let (_, index) = mounted(&ed).await?;
            let rel = p.path.replace('\\', "/");
            let to = p.to.unwrap_or_else(|| format!("gamedata/{rel}"));
            let (root, guard, overwrite) =
                (project.root().to_owned(), project.write_guard(None), p.overwrite);
            let outcome =
                ed.exec.io(move || copy_from_game(&index, &root, &rel, &to, overwrite, &guard)).await;
            match outcome {
                Ok((target, existed)) => {
                    if let Some(dir) = Path::new(&target).parent() {
                        project.tree.mark_fs(dir);
                    }
                    Ok(
                        json!({ "path": paths::wire(&rel_or_full(project.root(), &target)), "copied": true, "replaced": existed }),
                    )
                }
                Err((target, existed, why)) => Err(refuse_with(
                    why,
                    Map::from_iter([
                        ("path".to_owned(), json!(paths::wire(&rel_or_full(project.root(), &target)))),
                        ("copied".to_owned(), json!(false)),
                        ("exists".to_owned(), json!(existed)),
                    ]),
                )),
            }
        }
        ImportFrom::Library => {
            let rel = paths::to_backslash(p.path.trim()).trim_start_matches('\\').to_owned();
            let lower = rel.to_ascii_lowercase();
            // an object comes in with its textures and thumbnails
            let object = (lower.starts_with("rawdata\\objects\\")
                && lower.ends_with(".object")
                && p.to.is_none())
            .then(|| rel.get("rawdata\\objects\\".len()..rel.len() - ".object".len()).map(str::to_owned))
            .flatten();
            if let Some(r) = object {
                let job = library::bring_in(&ed, vec![r])?;
                let info = ed.jobs.wait(job).await;
                return match info {
                    Some(i) if i.error.is_none() => Ok(json!({
                        "path": paths::wire(&rel),
                        "copied": i.result.as_ref().and_then(|r| r.get("copied").cloned()).unwrap_or(Value::Null),
                    })),
                    Some(i) => Err(CmdError::new("failed", i.error.unwrap_or_default())),
                    None => Err(CmdError::internal("the copy job is gone")),
                };
            }
            let from = crate::games::library_root(&ed).join(&rel);
            let to_rel = paths::to_backslash(&p.to.unwrap_or_else(|| rel.clone()));
            let target = paths::join(project.root(), &to_rel);
            let guard = project.write_guard(None);
            let overwrite = p.overwrite;
            let t = target.clone();
            let done = ed
                .exec
                .io(move || -> Result<bool, String> {
                    if !from.is_file() {
                        return Err(format!(
                            "the library has no {}",
                            paths::wire(&from.display().to_string())
                        ));
                    }
                    guard.check(Path::new(&t)).map_err(|e| e.to_string())?;
                    let existed = Path::new(&t).exists();
                    if existed && !overwrite {
                        return Err("it is in the project already (overwrite=true replaces it)".to_owned());
                    }
                    if let Some(dir) = Path::new(&t).parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    std::fs::copy(&from, &t).map_err(|e| format!("it cannot be copied: {e}"))?;
                    Ok(existed)
                })
                .await
                .map_err(refuse)?;
            if let Some(dir) = Path::new(&target).parent() {
                project.tree.mark_fs(dir);
            }
            Ok(json!({ "path": paths::wire(&to_rel), "copied": true, "replaced": done }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn an_old_project_is_converted_once_and_its_project_ltx_goes() {
        let t = tempfile::tempdir().unwrap();
        let (root, home) = (t.path().join("Night"), t.path().join("home"));
        put(&root, "mod.ltx", "[module]\r\nid = night\r\n");
        put(&root, "rawdata/levels/a.level", "x");
        put(
            &root,
            "project.ltx",
            "[project]\r\nname=Night\r\nsdk_root=D:\\Editor\\\r\ngame_root=D:\\Games\\DA\\\r\nlast_scene=rawdata\\levels\\a.level\r\n[xms]\r\nexport_target=D:\\Out\r\n",
        );
        let first = prepare(&root, &home, &[]).unwrap();
        assert_eq!(first.file_path, root.join("Night.grizzly"));
        assert!(project::is_uuid(first.file.id()));
        assert_eq!(first.file.scene(), Some("rawdata/levels/a.level"));
        let old = first.legacy.clone().unwrap();
        assert_eq!((old.game_root.as_str(), old.export_target.as_str()), ("D:\\Games\\DA", "D:\\Out"));
        assert!(!root.join("project.ltx").exists(), "project.ltx is read once and goes");
        assert!(matches!(first.guide, Some(Ok(true))));
        assert!(first.state.agents_guide_written());
        // the project file names nothing outside the project
        let text = std::fs::read_to_string(&first.file_path).unwrap();
        assert!(!text.contains(":\\") && !text.contains("Editor") && !text.contains("DA"), "{text}");

        // what the open keeps of the state: the guide is never written again by itself
        std::fs::create_dir_all(home.join("projects")).unwrap();
        std::fs::write(machine::state_path(&home, first.file.id()), first.state.to_text()).unwrap();
        std::fs::remove_file(root.join(guide::FILE)).unwrap();
        let second = prepare(&root, &home, &[]).unwrap();
        assert_eq!(second.file.id(), first.file.id());
        assert!(second.legacy.is_none() && second.guide.is_none());
        assert!(!root.join(guide::FILE).exists());
        assert_eq!(
            std::fs::read_to_string(&second.file_path).unwrap(),
            text,
            "an unchanged file is not rewritten"
        );
    }

    #[test]
    fn a_fresh_clone_gets_its_project_file_and_a_broken_one_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let (root, home) = (t.path().join("Clone"), t.path().join("home"));
        put(&root, "mod.ltx", "[module]\r\nid = clone\r\n");
        let p = prepare(&root, &home, &[]).unwrap();
        assert_eq!(p.file_path, root.join("Clone.grizzly"));
        assert!(p.legacy.is_none() && p.problems.is_empty());
        // two project files: the first by name is the project, both are named
        put(&root, "Another.grizzly", "return { grizzly = 1, id = \"x\" }");
        let two = prepare(&root, &home, &[]).unwrap();
        assert_eq!(two.file_path, root.join("Another.grizzly"));
        assert!(two.problems.iter().any(|p| p.contains("2 project files")), "{:?}", two.problems);
        // one that does not read opens nothing
        put(&root, "Another.grizzly", "return { grizzly = 1, id = }");
        let broken = prepare(&root, &home, &[]).err().unwrap_or_default();
        assert!(broken.contains("Another.grizzly") && broken.contains("line 1"), "{broken}");
    }
}
