//! The raw files of the module's game data that no asset wraps get their assets in place by
//! themselves (`docs/ASSETS.md` section 6): when the project opens and whenever one appears, as a
//! job of the project. What [`crate::import::plan`] makes of them is what `asset.import` makes,
//! without `remove_descriptions`: a `.thm` stays, read by nobody. A file the import makes no
//! asset of (a picture the game cannot load by its name, no DDS) stays a file the build leaves
//! out, and nothing of the project is ever replaced. A commandlet does not watch; every module
//! build runs [`import_now`] first, so what ships never depends on the watcher's timing.

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use gz_base::RelPath;
use gz_editor::{Editor, ProjectSession};
use gz_project::asset_kind::MediaKind;
use gz_project::{ChangeKind, FileTree, Filter, Project, ProjectHooks, RAW_EXTENSIONS, is_raw};
use serde_json::json;
use smol_str::SmolStr;

use crate::import;
use crate::index::Index;
use crate::service::Media;

/// The job's kind and group.
pub const JOB_KIND: &str = "media_import";
const TITLE: &str = "Importing Media";

/// The folders of the module's game data whose raw files get their assets by themselves.
pub const FOLDERS: [&str; 3] = ["gamedata/textures", "gamedata/sounds", "gamedata/meshes"];

/// How long a burst of new files settles before one pass takes them all.
const SETTLE: Duration = Duration::from_millis(400);

/// The raw files under [`FOLDERS`] that no asset wraps: what the import is asked about. A `.thm`
/// beside a picture goes with the picture, a `#` companion with its texture. Pure.
#[must_use]
pub fn unwrapped(tree: &FileTree, index: &Index) -> Vec<RelPath> {
    let has = |b: &str| RelPath::new(b).is_ok_and(|r| tree.contains(&r));
    let taken = index.taken();
    let mut out = Vec::new();
    for folder in FOLDERS {
        let Ok(under) = RelPath::new(folder) else { continue };
        for (rel, entry) in tree.files_under(&under) {
            let s = rel.as_str();
            if !entry.is_file()
                || !is_raw(s, has)
                || gz_project::rules::is_media_asset_name(s)
                || taken.contains(&s.to_lowercase())
            {
                continue;
            }
            let Some((stem, ext)) = s.rsplit_once('.') else { continue };
            let beside_raw = RAW_EXTENSIONS.iter().any(|e| has(&format!("{stem}.{e}")));
            if ext.eq_ignore_ascii_case("thm") && beside_raw {
                continue;
            }
            if ext.eq_ignore_ascii_case("dds")
                && stem.strip_suffix('#').is_some_and(|main| has(&format!("{main}.dds")))
            {
                continue;
            }
            out.push(rel);
        }
    }
    out
}

/// The raw files under [`FOLDERS`] no asset wraps that the game reads as they are, project-relative
/// in lower case with `/`: what the import could not say in an asset the way the file says it (a
/// `.thm` or a sound's parameters), and what goes with such a file. They ship as they did before
/// there were assets; a picture the game cannot load (no DDS, a name it never asks for) and a
/// file of no game container do not. Blocking: it reads the headers of the textures.
#[must_use]
pub fn kept_as_files(root: &std::path::Path, tree: &FileTree, index: &Index) -> Vec<String> {
    let has = |b: &str| RelPath::new(b).is_ok_and(|r| tree.contains(&r));
    let taken = index.taken();
    let mut out = Vec::new();
    for folder in FOLDERS {
        let Ok(under) = RelPath::new(folder) else { continue };
        for (rel, entry) in tree.files_under(&under) {
            let key = rel.as_str().to_lowercase();
            if !entry.is_file() || !is_raw(rel.as_str(), has) || taken.contains(&key) {
                continue;
            }
            let ext = rel.extension().unwrap_or("").to_ascii_lowercase();
            let reads = match ext.as_str() {
                "thm" | "ogg" | "ogf" => true,
                "dds" => {
                    let name = crate::files::game_name_of(MediaKind::Texture, rel.as_str());
                    name.is_some_and(|n| crate::files::texture_name_reached(&n))
                        && crate::files::dds_head(&rel.to_fs(root))
                            .ok()
                            .is_some_and(|b| gz_fmt_tex::read_dds_info(&b).is_ok())
                }
                _ => false,
            };
            if reads {
                out.push(key);
            }
        }
    }
    out
}

/// What the import would write now: the raw files nothing wraps, planned, only what is not
/// there yet. `None` when it writes nothing.
async fn plan_now(ed: &Arc<Editor>, session: &Arc<ProjectSession>) -> Option<(Arc<Project>, import::Plan)> {
    let Ok((now, project)) = Project::of(ed) else { return None };
    if !Arc::ptr_eq(&now, session) {
        return None;
    }
    let media = Media::of(ed).await.ok()?;
    project.tree.refresh().await;
    let tree = project.tree.snapshot();
    let index = media.fresh_index().await;
    let root = session.root.clone();
    let guard = project.write_guard(None);
    let planned = ed
        .exec
        .io(move || {
            let from: Vec<PathBuf> = unwrapped(&tree, &index).iter().map(|r| r.to_fs(&root)).collect();
            if from.is_empty() {
                return None;
            }
            let module =
                if index.module.is_empty() { crate::files::module_id(&root) } else { index.module.clone() };
            let mut plan = import::plan(&from, None, &module, &root);
            // only what is new: a file of the project is never replaced
            plan.writes.retain(|(rel, _)| {
                let abs = rel.to_fs(&root);
                !abs.exists() && guard.check(&abs).is_ok()
            });
            (!plan.writes.is_empty()).then_some(plan)
        })
        .await;
    planned.map(|plan| (project, plan))
}

/// The import as a step of someone else's work (a module build): written at once, no job of its
/// own. Answers the assets it wrote.
pub async fn import_now(ed: &Arc<Editor>, session: &Arc<ProjectSession>) -> Result<Vec<String>, String> {
    // what a build of this project imports goes into this project only
    if Project::of(ed).ok().is_none_or(|(now, _)| !Arc::ptr_eq(&now, session)) {
        return Ok(Vec::new());
    }
    let Some((project, plan)) = plan_now(ed, session).await else { return Ok(Vec::new()) };
    import::write(ed, &project, &session.root, plan.writes, Vec::new()).await?;
    ed.log.info(format!("media import: {} asset(s) made in place", plan.assets.len()));
    Ok(plan.assets)
}

/// One pass: what is new written by a job the pass waits for. Nothing is done when the plan
/// writes nothing.
async fn pass(ed: &Arc<Editor>, session: &Arc<ProjectSession>) {
    let Some((project, plan)) = plan_now(ed, session).await else { return };
    // the job is the project's that was open when it starts: the one planned for, or none
    if Project::of(ed).ok().is_none_or(|(now, _)| !Arc::ptr_eq(&now, session)) {
        return;
    }
    let (writes, assets) = (plan.writes, plan.assets);
    let (work_ed, work_project, root) = (ed.clone(), project.clone(), session.root.clone());
    let started = ed.jobs.start(JOB_KIND, Some(JOB_KIND), TITLE, move |ctx| async move {
        ctx.stage(format!("Writing the assets of {} files", writes.len()));
        import::write(&work_ed, &work_project, &root, writes, Vec::new())
            .await
            .map_err(gz_task::JobError::new)?;
        Ok(json!({ "assets": assets }))
    });
    match started {
        Ok(id) => {
            ed.jobs.wait(id).await;
        }
        Err(busy) => ed.log.info(format!("media import: {busy}")),
    }
}

/// Whether a change may bring a raw file of [`FOLDERS`] that nothing wraps.
fn wanted(c: &gz_project::Change) -> bool {
    match c.kind {
        ChangeKind::Rescan => true,
        ChangeKind::Created | ChangeKind::Changed => {
            let key = c.path.as_str().to_ascii_lowercase();
            FOLDERS.iter().any(|f| key.starts_with(&format!("{f}/")))
        }
        ChangeKind::Removed => false,
    }
}

/// Follows the project's watcher while the project is open. It holds the editor and the session
/// only for a pass: a closed project is let go of, and its watcher's end ends this.
async fn follow(ed: Weak<Editor>, session: Weak<ProjectSession>) {
    let mut rx = {
        let (Some(editor), Some(open)) = (ed.upgrade(), session.upgrade()) else { return };
        let Ok((now, project)) = Project::of(&editor) else { return };
        if !Arc::ptr_eq(&now, &open) {
            return;
        }
        let exts: Vec<SmolStr> = RAW_EXTENSIONS.iter().chain(&["thm"]).map(SmolStr::new).collect();
        let rx = project.tree.changes().subscribe(Filter::Extensions(exts));
        drop(project);
        pass(&editor, &open).await;
        rx
    };
    while let Some(first) = rx.recv().await {
        let mut due = wanted(&first);
        loop {
            match tokio::time::timeout(SETTLE, rx.recv()).await {
                Ok(Some(c)) => due |= wanted(&c),
                Ok(None) => return,
                Err(_) => break,
            }
        }
        let (Some(editor), Some(open)) = (ed.upgrade(), session.upgrade()) else { return };
        if due {
            pass(&editor, &open).await;
        }
    }
}

/// Hangs the automatic import on every project that opens.
pub fn install(editor: &Editor) {
    ProjectHooks::of(editor).on_open(|ed, session| {
        ed.exec.spawn(follow(Arc::downgrade(ed), Arc::downgrade(session)));
    });
}
