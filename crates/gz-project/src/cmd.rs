//! The project area's commands. The wire is the C++ handlers': `GrizzlyMCP.cpp` (list_projects,
//! open_project), `EditorProject.cpp` (project_*, game_link), `EditorFileOps.cpp` (content_*),
//! `EditorGameContent.cpp` (darf_*), `LevelMcpContent.cpp` (content_create, list_assets).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath};
use gz_editor::{Args, ArgsExt, CmdError, Editor, ProjectSession, Registry, Reply};
use serde_json::{Map, Value, json};

use crate::delete::{self, DeleteContext, DeleteInfo};
use crate::hooks::{AssetLists, AssetTemplates, CATEGORIES, ProjectHooks, TemplateCx, TemplateKind};
use crate::list::{self, EntryState};
use crate::ops::{FileOps, Report};
use crate::project::{self, ProjectFile};
use crate::service::{FileCmd, ListCmd, Project, Projects};
use crate::{darf, fsys, paths, rules};

const AREA: &str = "gz-project";

/// The longest path the C++ editor built (`MAX_PATH`).
const MAX_PATH: usize = 260;

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "list_projects", list_projects);
    reg.add(AREA, "open_project", open_project);
    reg.add(AREA, "project_close", project_close);
    reg.add(AREA, "project_create", project_create);
    reg.add(AREA, "project_delete", project_delete);
    reg.add(AREA, "project_forget", project_forget);
    reg.add(AREA, "project_reveal", project_reveal);
    reg.add(AREA, "game_link", game_link);
    reg.add(AREA, "darf_list", darf_list);
    reg.add(AREA, "darf_copy", darf_copy);
    reg.add(AREA, "content_copy", content_copy);
    reg.add(AREA, "content_move", content_move);
    reg.add(AREA, "content_delete", content_delete);
    reg.add(AREA, "content_mkdir", content_mkdir);
    reg.add(AREA, "content_rename", content_rename);
    reg.add(AREA, "content_create", content_create);
    reg.add(AREA, "list_assets", list_assets);
}

/// A refusal: `{"ok":false,"error":<message>}`.
fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A refusal that carries fields of the reply after `error`.
fn refuse_with(message: impl Into<String>, fields: Map<String, Value>) -> CmdError {
    refuse(message).with_data(Value::Object(fields))
}

/// A path argument with forward slashes made backslashes.
fn arg_path(args: &Args, key: &str) -> Option<String> {
    args.text(key).map(paths::to_backslash)
}

fn editor_root_text(editor: &Editor) -> String {
    let mut root = paths::to_backslash(&editor.paths.root.to_string_lossy());
    if !root.ends_with('\\') {
        root.push('\\');
    }
    root
}

// ---- the project list -------------------------------------------------------------------------

async fn list_projects(ed: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let list = Projects::of(&ed).list(&ed).await;
    let entries = list.entries();
    let states = ed.exec.io(move || entries.into_iter().map(|e| (list::entry_state(&e), e)).collect::<Vec<_>>()).await;
    let projects: Vec<Value> = states
        .iter()
        .map(|(state, path)| json!({ "name": paths::wire(list::entry_name(path)), "path": paths::wire(path), "state": state.as_str() }))
        .collect();
    Ok(json!({ "projects": projects }))
}

// ---- open, close, create ----------------------------------------------------------------------

/// Whether the caller let unsaved work go: `discard`, or `-nodlg`, where the logged answer has
/// always been No.
fn lets_go(ed: &Editor, args: &Args, origin: Origin) -> bool {
    args.flag("discard") || origin == Origin::Unattended || ed.flags.unattended
}

async fn open_project(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let Some(path) = arg_path(&args, "path") else { return Err(CmdError::invalid("missing 'path' argument")) };
    if let Ok((_, open)) = Project::of(&ed)
        && (open.root().eq_ignore_ascii_case(&path) || paths::same_path(open.root(), &path))
    {
        return Ok(json!({ "note": "already open" }));
    }
    // -project <name> from the command line: not a folder, so the name of a listed project
    let mut folder = path.clone();
    let probe = PathBuf::from(&path);
    if origin != Origin::Mcp && !paths::is_absolute(&path) && !ed.exec.io(move || fsys::dir_exists(&probe)).await {
        let list = Projects::of(&ed).list(&ed).await;
        if let Some(listed) = list.entries().into_iter().find(|e| list::entry_name(e).eq_ignore_ascii_case(&path)) {
            folder = listed;
        }
    }
    open(&ed, &folder, lets_go(&ed, &args, origin)).await?;
    Ok(json!({}))
}

/// Opens the project at `folder`, closing the one that is open.
pub(crate) async fn open(ed: &Arc<Editor>, folder: &str, discard: bool) -> Result<(), CmdError> {
    let folder = paths::trim_trailing(folder.to_owned());
    let folder = folder.as_str();
    let probe = PathBuf::from(folder);
    if folder.is_empty() || !ed.exec.io(move || fsys::dir_exists(&probe)).await {
        return Err(refuse("project folder does not exist"));
    }
    if ed.project_opt().is_some() {
        ed.refuse_unsaved(discard)?;
        close_session(ed).await?;
    }

    let name = paths::leaf_of(folder).to_owned();
    let hooks = ProjectHooks::of(ed);
    let scaffolds = hooks.scaffolds();
    let (root, scaffold_name) = (PathBuf::from(folder), name.clone());
    let (file, gitignore, problems) = ed
        .exec
        .io(move || {
            // the module skeleton first: a project that predates it gets it silently
            let problems: Vec<String> = scaffolds.iter().filter_map(|s| s(&root, &scaffold_name).err()).collect();
            let file = match gz_editor::fsx::read_opt(&root.join(project::FILE)) {
                Ok(Some(bytes)) => ProjectFile::parse(&gz_base::text::decode_guess(&bytes)),
                _ => ProjectFile::default(),
            };
            (file, rules::ensure_gitignore(&root), problems)
        })
        .await;
    for p in problems {
        ed.log.warn(format!("the project's module skeleton: {p}"));
    }
    match gitignore {
        Ok(true) => ed.log.info("Project is a git repository: .gitignore written for the editor's own files"),
        Ok(false) => {}
        Err(e) => ed.log.warn(format!("the project's .gitignore could not be written: {e}")),
    }

    let stored_scene = file.last_scene().map(str::to_owned);
    let game_root = file.game_root().to_owned();
    let session = Arc::new(ProjectSession::new(PathBuf::from(folder)));
    let project = session.services.insert(Project::new(ed, folder, &name, file));
    if let Err(e) = project.edit(FileCmd::Manifest { name: name.clone(), sdk_root: editor_root_text(ed), game_root }).await {
        ed.log.warn(e.message);
    }

    // the scene to reopen: project-relative; an absolute one only inside this folder - a copied
    // project must never open, and then save into, the scene of the one it was copied from
    if let Some(stored) = stored_scene.filter(|s| !s.is_empty()) {
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
            None => ed.log.warn("project: the last scene lies outside the project and is not opened"),
            Some((rel, true)) => project.set_pending_scene(RelPath::new(&rel).ok()),
            // a stale path is skipped silently
            Some((_, false)) => {}
        }
    }

    ed.set_project(Some(session.clone()));
    remember_opened(ed, folder).await;
    ed.log.reset_counts();
    ed.log.info(format!("Project opened: {folder}"));
    hooks.run_open(ed, &session);
    Ok(())
}

/// The project list after `folder` was open: it goes first. A list that cannot be written is
/// logged; the project is open all the same.
async fn remember_opened(ed: &Arc<Editor>, folder: &str) {
    let projects = Projects::of(ed);
    let create_parent = Projects::create_parent(&projects.list(ed).await);
    if let Err(e) = projects.edit(ListCmd::Opened { project: folder.to_owned(), create_parent }).await {
        ed.log.warn(e.message);
    }
}

/// Closes the open project: its hooks, the scene to reopen, the list, then the session goes.
/// The caller settled unsaved work.
pub(crate) async fn close_session(ed: &Arc<Editor>) -> Result<(), CmdError> {
    let Some(session) = ed.project_opt() else { return Ok(()) };
    ProjectHooks::of(ed).run_close(ed, &session);
    if let Some(project) = session.services.get::<Project>() {
        let root = project.root().to_owned();
        let scene = project.scene_to_remember();
        let check_root = root.clone();
        // a scene from outside the project is not reopened
        let rel = ed.exec.io(move || scene.and_then(|s| project::scene_in_project(&check_root, &s))).await;
        if let Err(e) = project.edit(FileCmd::LastScene(rel)).await {
            ed.log.warn(e.message);
        }
        remember_opened(ed, &root).await;
    }
    ed.set_project(None);
    Ok(())
}

async fn project_close(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(refuse("no project is open: the Start page is up"));
    }
    let save = args.flag("save");
    let mut discard = args.flag("discard");
    if save && discard {
        return Err(refuse("save and discard exclude each other"));
    }
    let dirty = ed.dirty_docs();
    let dirty_json = serde_json::to_value(&dirty).unwrap_or(Value::Null);
    if !dirty.is_empty() {
        if save {
            // oracle: the C++ saved each tab here; no area of this build can save on request yet
            return Err(refuse_with(
                "unsaved work cannot be saved from project_close in this build: save each document, or pass discard=true",
                Map::from_iter([("dirty".to_owned(), dirty_json)]),
            ));
        }
        if !discard {
            if origin != Origin::Unattended && !ed.flags.unattended {
                return Err(CmdError::unsaved(dirty_json.clone()).with_data(json!({ "dirty": dirty_json, "note": "pass save=true or discard=true" })));
            }
            discard = true;
        }
    }
    close_session(&ed).await?;
    let mut out = Map::new();
    out.insert("start_page".into(), json!(true));
    if discard && !dirty.is_empty() {
        out.insert("discarded".into(), dirty_json);
    }
    Ok(Value::Object(out))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

async fn project_create(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let name = args.str("name").unwrap_or_default().to_owned();
    let projects = Projects::of(&ed);
    let list = projects.list(&ed).await;
    // no parent: the location the Start page's form shows
    let parent = paths::trim_trailing(arg_path(&args, "parent").unwrap_or_else(|| Projects::create_parent(&list)));
    if ed.project_opt().is_some() {
        return Err(refuse("a project is open: project_close first, then create"));
    }
    if parent.len() + name.len() + 2 > MAX_PATH {
        return Err(refuse("the path is too long"));
    }
    let folder = paths::join(&parent, &name);
    let probe = PathBuf::from(&folder);
    let existed = ed.exec.io(move || fsys::dir_exists(&probe) && list::has_manifest(&probe)).await;

    if !valid_name(&name) {
        return Err(refuse("The name may hold Latin letters, digits, _ and - only."));
    }
    // a relative location would land in the editor's working folder
    if !paths::is_absolute(&parent) {
        return Err(refuse("The location must be a full path."));
    }
    if !existed {
        let target = PathBuf::from(&folder);
        let made = ed.exec.io(move || std::fs::create_dir_all(&target).map_err(|e| fsys::win32(&e)).map(|()| fsys::dir_exists(&target))).await;
        match made {
            Ok(true) => {}
            Ok(false) => return Err(refuse(format!("The folder cannot be created: {folder} (win32 0)."))),
            Err(code) => return Err(refuse(format!("The folder cannot be created: {folder} (win32 {code})."))),
        }
    }
    // a project there already is simply opened; the skeleton is written by the open's hooks
    open(&ed, &folder, false).await?;
    Ok(json!({ "path": paths::wire(&folder), "name": name, "existed": existed, "opened": true }))
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
    m.insert("failed".into(), d.failed.iter().map(|f| json!({ "path": paths::wire(&f.path), "reason": paths::wire(&f.reason) })).collect());
    m
}

/// What the delete checks compare with, gathered on the IO pool.
async fn delete_context(ed: &Arc<Editor>) -> DeleteContext {
    let listed = Projects::of(ed).list(ed).await.entries();
    let open = Project::of(ed).ok().map(|(_, p)| (p.root().to_owned(), p.game_root()));
    let (exe, root) = (ed.paths.exe_dir.clone(), ed.paths.root.clone());
    ed.exec
        .io(move || {
            let guards = delete::system_guards(&exe, &root, open.as_ref().map(|o| o.1.as_str()), &listed);
            DeleteContext { listed, open: open.map(|o| o.0), guards }
        })
        .await
}

async fn project_delete(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Some(path) = arg_path(&args, "path") else { return Err(refuse("missing 'path' - a project folder from list_projects")) };
    // the caller decided: no confirmation on this path, the same checks as the window's
    let cx = delete_context(&ed).await;
    let outcome = ed.exec.io(move || delete::delete_project(&cx, &path)).await;
    match outcome {
        Ok(mut info) => {
            Projects::of(&ed).edit(ListCmd::Forget(info.path.clone())).await?;
            info.removed_from_list = true;
            if info.missing {
                ed.log.info(format!("Project list: {} removed, the folder does not exist any more", info.path));
            } else {
                ed.log.info(format!("Project deleted: {} ({} files, {} folders, {} links)", info.path, info.files, info.folders, info.links));
            }
            let mut out = Map::new();
            if info.missing {
                out.insert("note".into(), json!("the folder does not exist any more - removed from the list only"));
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
            Err(refuse_with(failed.message, fields))
        }
    }
}

async fn project_forget(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Some(path) = arg_path(&args, "path") else { return Err(refuse("missing 'path' - a folder from list_projects")) };
    let projects = Projects::of(&ed);
    let list = projects.list(&ed).await;
    let Some(listed) = list.find_listed(&path) else { return Err(refuse("It is not in the project list.")) };
    let probe = listed.clone();
    if ed.exec.io(move || list::entry_state(&probe)).await == EntryState::Project {
        return Err(refuse("The folder still holds a project, so it stays in the list."));
    }
    projects.edit(ListCmd::Forget(listed)).await?;
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

async fn project_reveal(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Some(path) = arg_path(&args, "path") else { return Err(refuse("missing 'path' - a project folder from list_projects")) };
    let list = Projects::of(&ed).list(&ed).await;
    let Some(folder) = list.find_listed(&path) else { return Err(refuse("It is not in the project list - only a listed project is shown from here.")) };
    let probe = PathBuf::from(&folder);
    if !ed.exec.io(move || fsys::dir_exists(&probe)).await {
        return Err(refuse("The folder does not exist any more."));
    }
    // dry_run answers the folder and opens nothing: a test leaves no window behind
    let dry = args.flag("dry_run");
    if !dry {
        launch_explorer(&folder).map_err(refuse)?;
    }
    Ok(json!({ "path": paths::wire(&folder), "opened": !dry, "dry_run": dry }))
}

// ---- the linked game ----------------------------------------------------------------------------

/// Links the open project to `folder` and mounts its content. The reason on failure.
async fn link_game(ed: &Arc<Editor>, project: &Arc<Project>, folder: &str) -> Result<(), String> {
    let clean = paths::trim_trailing(paths::to_backslash(folder)).trim_end_matches('\\').to_owned();
    let current = project.game_root();
    if !current.is_empty() && paths::same_path(&current, &clean) {
        let probe = clean.clone();
        return ed.exec.io(move || darf::looks_like_game(&probe)).await.map_err(str::to_owned);
    }
    let probe = clean.clone();
    let index = ed.exec.io(move || darf::DarfIndex::mount(&probe)).await?;
    for skipped in &index.skipped {
        ed.log.warn(format!("DARF: '{skipped}' is not a readable archive, skipped"));
    }
    ed.log.info(format!("DARF content mounted: {} archives, {} files ({})", index.archives.len(), index.items.len(), index.root));
    project.put_darf(Arc::new(index));
    project.edit(FileCmd::GameRoot(clean.clone())).await.map_err(|e| e.message)?;
    ed.log.info(format!("Game linked: {clean}"));
    Ok(())
}

/// `{linked, game_root, reason}` of the open project.
async fn link_fields(ed: &Arc<Editor>, project: &Arc<Project>, reason: &str) -> Map<String, Value> {
    let p = project.clone();
    let linked = ed.exec.io(move || p.game_linked()).await;
    let mut m = Map::new();
    m.insert("linked".into(), json!(linked));
    m.insert("game_root".into(), json!(paths::wire(&project.game_root())));
    m.insert("reason".into(), json!(reason));
    m
}

async fn game_link(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Ok((_, project)) = Project::of(&ed) else {
        let reason = "no project is open";
        let fields = Map::from_iter([("linked".to_owned(), json!(false)), ("game_root".to_owned(), json!("")), ("reason".to_owned(), json!(reason))]);
        return Err(refuse_with(reason, fields));
    };
    if let Some(path) = arg_path(&args, "path") {
        return match link_game(&ed, &project, &path).await {
            Ok(()) => Ok(Value::Object(link_fields(&ed, &project, "").await)),
            Err(reason) => {
                let fields = link_fields(&ed, &project, &reason).await;
                Err(refuse_with(reason, fields))
            }
        };
    }
    // a report: what is wrong with the link is a reason, not a failure
    let root = project.game_root();
    let reason = if root.is_empty() {
        "no game linked".to_owned()
    } else {
        ed.exec.io(move || darf::looks_like_game(&root).err().unwrap_or_default().to_owned()).await
    };
    Ok(Value::Object(link_fields(&ed, &project, &reason).await))
}

/// The linked game's listing of the open project, mounted on first need.
async fn mounted(ed: &Arc<Editor>) -> Result<(Arc<Project>, Arc<darf::DarfIndex>), CmdError> {
    let Ok((_, project)) = Project::of(ed) else { return Err(refuse("no project is open")) };
    let p = project.clone();
    let index = ed.exec.io(move || p.darf_index()).await.map_err(refuse)?;
    Ok((project, index))
}

async fn darf_list(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let (_, index) = mounted(&ed).await?;
    let filter = args.str("filter").unwrap_or_default().to_lowercase();
    let ext = args.str("ext").unwrap_or_default().to_lowercase();
    let ext = ext.strip_prefix('.').unwrap_or(&ext).to_owned();
    let limit = args.i64("limit").filter(|&v| v > 0).unwrap_or(200);
    let (mut total, mut shown) = (0i64, 0i64);
    let mut files = Vec::new();
    for it in &index.items {
        if !filter.is_empty() && !it.path.contains(&filter) {
            continue;
        }
        if !ext.is_empty() {
            let Some(e) = it.path.rfind('.').and_then(|d| it.path.get(d + 1..)) else { continue };
            if !e.eq_ignore_ascii_case(&ext) {
                continue;
            }
        }
        total += 1;
        if shown >= limit {
            continue;
        }
        files.push(json!({ "path": paths::wire(&it.path), "size": it.size, "source": it.source_name() }));
        shown += 1;
    }
    Ok(json!({ "shown": shown, "total": total, "files": files }))
}

async fn darf_copy(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let (project, index) = mounted(&ed).await?;
    let Some(path) = args.text("path").map(str::to_owned) else { return Err(refuse("missing 'path' argument")) };
    let overwrite = args.flag("overwrite");
    let root = project.root().to_owned();
    let outcome = ed.exec.io(move || index.copy_to_project(&root, &path, overwrite)).await;
    match outcome {
        Ok((target, existed)) => {
            if let Some(dir) = Path::new(&target).parent() {
                project.tree.mark_fs(dir);
            }
            Ok(json!({ "target": paths::wire(&target), "copied": true, "exists": existed }))
        }
        Err((target, existed, why)) => {
            let fields = Map::from_iter([
                ("target".to_owned(), json!(paths::wire(&target))),
                ("copied".to_owned(), json!(false)),
                ("exists".to_owned(), json!(existed)),
            ]);
            Err(refuse_with(why, fields))
        }
    }
}

// ---- the project's files ------------------------------------------------------------------------

/// The open project for a content command, or the C++'s `no project is open`.
fn content_project(ed: &Editor) -> Result<Arc<Project>, CmdError> {
    Project::of(ed).map(|(_, p)| p).map_err(|_| refuse("no project is open"))
}

/// The entries of a `;`-separated list, trimmed.
fn entry_list(raw: &str) -> Vec<String> {
    raw.split(';').map(|e| e.trim_matches(' ')).filter(|e| !e.is_empty()).map(str::to_owned).collect()
}

/// `{ok, files, folders, skipped, failed, errors, skips}`; a report with failures is `ok:false`.
fn report_reply(rep: &Report) -> Reply {
    let fields = rep.fields();
    if rep.ok() {
        return Ok(Value::Object(fields));
    }
    let message = format!("{} {} failed", rep.failed, if rep.failed == 1 { "entry" } else { "entries" });
    Err(refuse_with(message, fields))
}

#[derive(Clone, Copy)]
enum Transfer {
    Copy,
    Move,
}

async fn transfer(ed: Arc<Editor>, args: Args, op: Transfer) -> Reply {
    let project = content_project(&ed)?;
    let Some(src) = args.text("src").map(str::to_owned) else { return Err(refuse("missing 'src' argument (';'-separated list allowed)")) };
    let Some(dst) = args.text("dst").map(str::to_owned) else { return Err(refuse("missing 'dst' argument (target folder inside the project)")) };
    let recursive = args.bool("recursive").unwrap_or(true);
    let overwrite = args.flag("overwrite");
    let p = project.clone();
    let (rep, touched) = ed
        .exec
        .io(move || {
            let darf = || p.darf_index().ok();
            let mut ops = FileOps::new(p.root(), &darf);
            let mut rep = Report::default();
            for entry in entry_list(&src) {
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

async fn content_copy(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    transfer(ed, args, Transfer::Copy).await
}

async fn content_move(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    transfer(ed, args, Transfer::Move).await
}

fn no_game() -> Option<Arc<darf::DarfIndex>> {
    None
}

async fn content_delete(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let project = content_project(&ed)?;
    let Some(list) = args.text("path").map(str::to_owned) else { return Err(refuse("missing 'path' argument (';'-separated list allowed)")) };
    let recursive = args.flag("recursive");
    let p = project.clone();
    let (rep, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(p.root(), &no_game);
            let mut rep = Report::default();
            for entry in entry_list(&list) {
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

async fn content_mkdir(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let project = content_project(&ed)?;
    let Some(path) = args.text("path").map(str::to_owned) else { return Err(refuse("missing 'path' argument (folder inside the project)")) };
    let p = project.clone();
    let (made, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(p.root(), &no_game);
            (ops.mkdir(&path), ops.touched)
        })
        .await;
    for dir in touched {
        project.tree.mark_fs(Path::new(&dir));
    }
    let full = made.map_err(refuse)?;
    Ok(json!({ "path": paths::wire(&full) }))
}

async fn content_rename(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let project = content_project(&ed)?;
    let (Some(path), Some(name)) = (args.text("path").map(str::to_owned), args.text("name").map(str::to_owned)) else {
        return Err(refuse("missing 'path' (file or folder inside the project) or 'name' (the new leaf name)"));
    };
    let p = project.clone();
    let (renamed, touched) = ed
        .exec
        .io(move || {
            let mut ops = FileOps::new(p.root(), &no_game);
            (ops.rename(&path, &name), ops.touched)
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
    Ok(json!({ "path": paths::wire(&to) }))
}

async fn content_create(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    const KINDS: &str = "kind must be quest, behaviour, character or pda";
    let kind_text = args.str("kind").unwrap_or_default().to_owned();
    if kind_text.is_empty() {
        return Err(refuse(KINDS));
    }
    let Ok((_, project)) = Project::of(&ed) else { return Err(refuse("no active project")) };
    let Some(kind) = TemplateKind::parse(&kind_text) else { return Err(refuse(KINDS)) };
    // oracle: the C++ made it in the folder its content browser had open; the browser's state
    // is not this crate's, so the caller names the folder (project-relative, the root without)
    let folder = match args.text("folder") {
        Some(f) => RelPath::new(f).map_err(|e| refuse(e.to_string()))?,
        None => RelPath::root(),
    };
    let Some(template) = AssetTemplates::of(&ed).get(kind) else {
        return Err(refuse(format!("Cannot create the {}: this build has no {} template", kind.noun(), kind.key())));
    };
    let root = PathBuf::from(project.root());
    let rel_folder = folder.clone();
    let made = ed
        .exec
        .io(move || -> Result<RelPath, String> {
            let dir = rel_folder.to_fs(&root);
            if !fsys::dir_exists(&dir) {
                return Err(format!("Cannot create the {}: the folder {} does not exist", kind.noun(), rel_folder.to_backslash()));
            }
            let found = (1..=999u32).map(|n| (n, kind.leaf(n))).find(|(_, leaf)| std::fs::symlink_metadata(dir.join(leaf)).is_err());
            let Some((n, leaf)) = found else { return Err(kind.too_many().to_owned()) };
            let stem = leaf.rfind('.').and_then(|d| leaf.get(..d)).unwrap_or(&leaf).to_owned();
            let bytes = template(&TemplateCx { stem: &stem, n }).map_err(|e| format!("Cannot create the {}: {e}", kind.noun()))?;
            gz_editor::fsx::write_atomic(&dir.join(&leaf), &bytes).map_err(|e| format!("Cannot create the {}: {e}", kind.noun()))?;
            rel_folder.join(&leaf).map_err(|e| e.to_string())
        })
        .await
        .map_err(refuse)?;
    project.tree.mark(folder);
    Ok(json!({ "kind": kind_text, "made": made.to_backslash() }))
}

/// `Levels`: the project's scenes (`rawdata\levels`, names without `.level`, as `$maps$`
/// lists them) and the editor's base scenes the project does not hold.
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
    let root = ed.paths.root.clone();
    let base = ed.exec.io(move || project::base_scenes(&root)).await;
    for b in base {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&b)) {
            names.push(b);
        }
    }
    names
}

async fn list_assets(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    // Objects by default: the only placeable category
    let category = match args.text("category") {
        Some(c) => match CATEGORIES.iter().find(|n| n.eq_ignore_ascii_case(c)) {
            Some(n) => *n,
            None => {
                let fields = Map::from_iter([("categories".to_owned(), json!(CATEGORIES))]);
                return Err(refuse_with("unknown category", fields));
            }
        },
        None => "Objects",
    };
    let items = if category == "Levels" {
        level_names(&ed).await
    } else {
        let Some(provider) = AssetLists::of(&ed).get(category) else { return Err(refuse("category has no enumerator in this build")) };
        let editor = ed.clone();
        ed.exec.io(move || provider(&editor)).await
    };
    let filter = args.str("filter").unwrap_or_default().to_lowercase();
    let limit = args.i64("limit").unwrap_or(200);
    let (mut total, mut shown) = (0i64, 0i64);
    let mut assets = Vec::new();
    for name in &items {
        if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
            continue;
        }
        total += 1;
        if shown >= limit {
            continue;
        }
        assets.push(json!({ "name": name }));
        shown += 1;
    }
    Ok(json!({ "category": category, "shown": shown, "total": total, "assets": assets }))
}
