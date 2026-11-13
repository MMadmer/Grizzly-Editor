//! What a scene takes from outside the project (`docs/ASSETS.md` section 9).
//!
//! A scene's objects, groups and textures resolve in this order: the project, the game, the
//! editor's library (Preferences > Library: a folder holding `rawdata` and `gamedata`, by default
//! the editor's own). What the author places from the library is copied into the project as it is
//! placed ([`bring_in`]): the object, its textures and thumbnails, under the same relative paths.
//! What a scene still takes from the library is named in the project file's `library` list
//! ([`note_scene_refs`], called when a scene is saved), and [`collect`] copies all of it in.
//!
//! An object reference is spelled as the library spells it: `props\box_wood_01`, the path below
//! `rawdata\objects` without the extension.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::fsx::WriteGuard;
use gz_editor::{CmdError, Editor, JobError, JobId};
use serde::Serialize;

use crate::games::library_root;
use crate::paths;
use crate::service::Project;

/// Where a reference resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolves {
    Project,
    Library,
    Nowhere,
}

fn object_rel(r: &str) -> PathBuf {
    let r = paths::to_backslash(r.trim());
    let r = r.trim_start_matches('\\');
    Path::new("rawdata").join("objects").join(format!("{r}.object"))
}

/// Where the object `r` resolves: the project first, then the library. (A game holds no editable
/// objects.) A few attribute reads.
#[must_use]
pub fn resolve_object(project: &Path, library: &Path, r: &str) -> Resolves {
    let rel = object_rel(r);
    if project.join(&rel).is_file() {
        Resolves::Project
    } else if library.join(&rel).is_file() {
        Resolves::Library
    } else {
        Resolves::Nowhere
    }
}

/// One file to copy into the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub from: PathBuf,
    /// Project-relative.
    pub to: PathBuf,
    pub bytes: u64,
}

/// What bringing references in comes to, before anything is written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub copies: Vec<Copy>,
    /// References neither the project nor the library has: nothing to copy.
    pub missing: Vec<String>,
}

impl Plan {
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.copies.iter().map(|c| c.bytes).sum()
    }
}

/// The textures an object's surfaces name, lower case, backslashes, no extension.
fn object_textures(bytes: &[u8]) -> Vec<String> {
    let Ok(object) = gz_fmt_object::Object::read(bytes) else { return Vec::new() };
    let mut out: Vec<String> = object
        .surfaces
        .iter()
        .map(|s| paths::to_backslash(s.texture.trim()).to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// What copying the objects `refs` into the project takes: each object with its thumbnail, and
/// the textures it names (`rawdata\textures\*.tga` and `.thm`, `gamedata\textures\*.dds` and
/// `.thm`) - whatever of it the library has. A file the project already holds is skipped, and a
/// texture the game holds (`game_has(<path>)` with forward slashes, `textures/x/y.dds`) is not
/// copied at all. Blocking: reads the library's objects.
#[must_use]
pub fn plan(library: &Path, project: &Path, refs: &[String], game_has: &dyn Fn(&str) -> bool) -> Plan {
    let mut out = Plan::default();
    let add = |out: &mut Plan, rel: PathBuf| {
        if out.copies.iter().any(|c| c.to == rel) || project.join(&rel).exists() {
            return;
        }
        let from = library.join(&rel);
        if let Ok(meta) = std::fs::metadata(&from)
            && meta.is_file()
        {
            out.copies.push(Copy { from, to: rel, bytes: meta.len() });
        }
    };
    let mut textures: Vec<String> = Vec::new();
    for r in refs {
        let rel = object_rel(r);
        let from = library.join(&rel);
        if !project.join(&rel).is_file() && !from.is_file() {
            out.missing.push(r.clone());
            continue;
        }
        let bytes = std::fs::read(project.join(&rel)).or_else(|_| std::fs::read(&from)).unwrap_or_default();
        textures.extend(object_textures(&bytes));
        add(&mut out, rel.clone());
        add(&mut out, rel.with_extension("thm"));
    }
    textures.sort();
    textures.dedup();
    for t in textures {
        if game_has(&format!("textures/{}.dds", t.replace('\\', "/"))) {
            continue;
        }
        for (top, ext) in [("rawdata", "tga"), ("rawdata", "thm"), ("gamedata", "dds"), ("gamedata", "thm")] {
            add(&mut out, Path::new(top).join("textures").join(format!("{t}.{ext}")));
        }
    }
    out
}

/// Copies what `plan` names into the project at `root`, every target through `guard`, stopping
/// between files when `stop` says so. Answers the files copied. Blocking.
pub fn copy(
    plan: &Plan,
    root: &Path,
    guard: &WriteGuard,
    mut each: impl FnMut(usize) -> Result<(), JobError>,
) -> Result<usize, JobError> {
    let mut done = 0;
    for (i, c) in plan.copies.iter().enumerate() {
        each(i)?;
        let to = root.join(&c.to);
        guard.check(&to).map_err(|e| JobError::new(e.to_string()))?;
        if to.exists() {
            continue;
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| JobError::new(format!("{} could not be made: {e}", dir.display())))?;
        }
        std::fs::copy(&c.from, &to)
            .map_err(|e| JobError::new(format!("{} could not be copied: {e}", c.to.display())))?;
        done += 1;
    }
    Ok(done)
}

/// The plan of `refs` for the open project, worked out on the IO pool.
pub async fn plan_for(editor: &Arc<Editor>, refs: Vec<String>) -> Result<Plan, CmdError> {
    let (_, project) = Project::of(editor)?;
    let library = library_root(editor);
    let p = project.clone();
    Ok(editor
        .exec
        .io(move || {
            let game = p.game_files().ok();
            let has = |path: &str| game.as_ref().is_some_and(|g| g.exists(path));
            plan(&library, Path::new(p.root()), &refs, &has)
        })
        .await)
}

/// Copies the library objects `refs`, their textures and thumbnails into the open project, as a
/// job: what the author placed from the library is in the project from then on. Answers the job.
pub fn bring_in(editor: &Arc<Editor>, refs: Vec<String>) -> Result<JobId, CmdError> {
    let (_, project) = Project::of(editor)?;
    let ed = editor.clone();
    let title = if refs.len() == 1 {
        format!("Copying {} into the project", refs[0])
    } else {
        format!("Copying {} library objects into the project", refs.len())
    };
    editor
        .jobs
        .start("library_bring_in", Some("library"), title, move |ctx| async move {
            ctx.stage("Looking at the library");
            let plan = plan_for(&ed, refs).await.map_err(|e| JobError::new(e.message))?;
            let total = plan.copies.len();
            let root = PathBuf::from(project.root());
            let guard = project.write_guard(None);
            let job = ctx.clone();
            let plan2 = plan.clone();
            let copied = ed
                .exec
                .io(move || {
                    copy(&plan2, &root, &guard, |i| {
                        job.progress(i, total);
                        job.check().map_err(JobError::from)
                    })
                })
                .await?;
            project.tree.mark_fs(Path::new(project.root()));
            Ok(serde_json::json!({ "copied": copied, "bytes": plan.bytes(), "missing": plan.missing }))
        })
        .map_err(|e| CmdError::new("busy", e.to_string()))
}

/// What a scene just saved references that only the library has: the project file's `library`
/// list takes it in, and loses what now resolves in the project. The scene's save calls it with
/// every object reference of the scene.
pub async fn note_scene_refs(editor: &Arc<Editor>, refs: Vec<String>) -> Result<(), CmdError> {
    let (_, project) = Project::of(editor)?;
    let library = library_root(editor);
    // the file as the disk has it now: the author or an agent may have edited it
    let listed = project.edit_file(|f| f).await?.library();
    let root = PathBuf::from(project.root());
    let next = editor
        .exec
        .io(move || {
            let mut next: Vec<String> = Vec::new();
            for r in listed.iter().chain(refs.iter()) {
                let r = paths::to_backslash(r.trim());
                if r.is_empty() || next.iter().any(|n| n.eq_ignore_ascii_case(&r)) {
                    continue;
                }
                if resolve_object(&root, &library, &r) == Resolves::Library {
                    next.push(r);
                }
            }
            next.sort_by_key(|r| r.to_ascii_lowercase());
            next
        })
        .await;
    project.edit_file(|f| f.with_library(&next)).await.map(|_| ())
}

/// What [`collect`] answers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Collected {
    pub files: usize,
    pub bytes: u64,
    /// The references neither the project nor the library has.
    pub missing: Vec<String>,
    /// The references copied (the list as it was).
    pub refs: Vec<String>,
}

/// Brings in everything the project file's `library` list names; afterwards the list holds only
/// what no library has. `dry_run`: the files and bytes it would copy, nothing written.
pub async fn collect(editor: &Arc<Editor>, dry_run: bool) -> Result<(Collected, Option<JobId>), CmdError> {
    let (_, project) = Project::of(editor)?;
    let refs = project.edit_file(|f| f).await?.library();
    let plan = plan_for(editor, refs.clone()).await?;
    let answer =
        Collected { files: plan.copies.len(), bytes: plan.bytes(), missing: plan.missing.clone(), refs };
    if dry_run {
        return Ok((answer, None));
    }
    let ed = editor.clone();
    let job = editor
        .jobs
        .start("library_collect", Some("library"), "Collecting library assets", move |ctx| async move {
            let total = plan.copies.len();
            let root = PathBuf::from(project.root());
            let guard = project.write_guard(None);
            let job = ctx.clone();
            let plan2 = plan.clone();
            ctx.stage("Copying the library assets");
            let copied = ed
                .exec
                .io(move || {
                    copy(&plan2, &root, &guard, |i| {
                        job.progress(i, total);
                        job.check().map_err(JobError::from)
                    })
                })
                .await?;
            project.tree.mark_fs(Path::new(project.root()));
            // what is left on the list is what no library has
            let left = plan.missing.clone();
            project.edit_file(|f| f.with_library(&left)).await.map_err(|e| JobError::new(e.message))?;
            Ok(serde_json::json!({ "copied": copied, "bytes": plan.bytes(), "missing": plan.missing }))
        })
        .map_err(|e| CmdError::new("busy", e.to_string()))?;
    Ok((answer, Some(job)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn a_library_object_comes_in_with_its_thumbnail_and_what_the_project_lacks() {
        let t = tempfile::tempdir().unwrap();
        let (lib, project) = (t.path().join("lib"), t.path().join("project"));
        // the object files are harmless text: no surfaces to read, so no textures
        put(&lib, "rawdata/objects/props/box.object", b"box");
        put(&lib, "rawdata/objects/props/box.thm", b"thumb");
        put(&lib, "rawdata/objects/props/crate.object", b"crate");
        put(&project, "rawdata/objects/props/crate.object", b"mine");
        let refs = vec!["props\\box".to_owned(), "props/crate".to_owned(), "props\\gone".to_owned()];
        let p = plan(&lib, &project, &refs, &|_| false);
        let to: Vec<String> =
            p.copies.iter().map(|c| c.to.display().to_string().replace('/', "\\")).collect();
        assert_eq!(to, ["rawdata\\objects\\props\\box.object", "rawdata\\objects\\props\\box.thm"]);
        assert_eq!(p.bytes(), 8);
        assert_eq!(p.missing, ["props\\gone"]);
        assert_eq!(resolve_object(&project, &lib, "props\\box"), Resolves::Library);
        assert_eq!(resolve_object(&project, &lib, "props\\crate"), Resolves::Project);
        assert_eq!(resolve_object(&project, &lib, "props\\gone"), Resolves::Nowhere);

        let copied = copy(&p, &project, &WriteGuard::new("", None), |_| Ok(())).unwrap();
        assert_eq!(copied, 2);
        assert_eq!(std::fs::read(project.join("rawdata/objects/props/box.thm")).unwrap(), b"thumb");
        assert_eq!(std::fs::read(project.join("rawdata/objects/props/crate.object")).unwrap(), b"mine");
        assert_eq!(resolve_object(&project, &lib, "props\\box"), Resolves::Project);
        // a second time there is nothing to copy
        assert!(plan(&lib, &project, &refs, &|_| false).copies.is_empty());
    }

    #[test]
    fn the_textures_an_object_names_come_with_it_unless_the_game_has_them() {
        let t = tempfile::tempdir().unwrap();
        let (lib, project) = (t.path().join("lib"), t.path().join("project"));
        let mut object = gz_fmt_object::Object::default();
        object.surfaces.push(gz_fmt_object::Surface {
            name: "box".into(),
            texture: "props\\Box_D".into(),
            shader: "default".into(),
            shader_xrlc: "default".into(),
            game_mtl: "default".into(),
            ..Default::default()
        });
        let p = lib.join("rawdata/objects/props/box.object");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, object.write()).unwrap();
        let back = gz_fmt_object::Object::read(&object.write()).map(|o| o.surfaces.len());
        assert_eq!(back.as_ref().ok(), Some(&1), "{back:?}");
        put(&lib, "rawdata/textures/props/box_d.tga", b"tga");
        put(&lib, "rawdata/textures/props/box_d.thm", b"thm");
        put(&lib, "gamedata/textures/props/box_d.dds", b"dds");
        let refs = ["props\\box".to_owned()];
        let mut to: Vec<String> = plan(&lib, &project, &refs, &|_| false)
            .copies
            .iter()
            .map(|c| c.to.display().to_string().replace('\\', "/"))
            .collect();
        to.sort();
        assert_eq!(
            to,
            [
                "gamedata/textures/props/box_d.dds",
                "rawdata/objects/props/box.object",
                "rawdata/textures/props/box_d.tga",
                "rawdata/textures/props/box_d.thm",
            ]
        );
        // the game holds the texture: the object comes alone
        let with_game = plan(&lib, &project, &refs, &|path| path == "textures/props/box_d.dds");
        assert_eq!(with_game.copies.len(), 1, "{:?}", with_game.copies);
    }

    #[test]
    fn a_stop_between_files_stops() {
        let t = tempfile::tempdir().unwrap();
        let (lib, project) = (t.path().join("lib"), t.path().join("project"));
        put(&lib, "rawdata/objects/a.object", b"a");
        put(&lib, "rawdata/objects/b.object", b"b");
        let p = plan(&lib, &project, &["a".to_owned(), "b".to_owned()], &|_| false);
        let stopped = copy(&p, &project, &WriteGuard::new("", None), |i| {
            if i == 1 { Err(JobError::new("Stopped.")) } else { Ok(()) }
        });
        assert!(stopped.is_err());
        assert!(project.join("rawdata/objects/a.object").is_file());
        assert!(!project.join("rawdata/objects/b.object").exists());
    }
}
