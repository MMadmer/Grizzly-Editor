//! Writing a build. Blocking: everything here runs on the IO pool.
//!
//! The module build is CLEAN: what ends up in the target is exactly this build and nothing a
//! previous one left. It is written beside the last one, in `<modules>\.new\<id>` (the game
//! reads modules one level deep, so nothing in there is a module), and takes its place in two
//! renames at the very end: a build that fails or is stopped costs the module nothing, and a
//! game started meanwhile never meets half of one. A file the last build holds unchanged (size
//! and write time) is a hard link to it: a rebuild costs seconds and no second copy on the disk.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gz_base::RelPath;
use gz_mod::{Manifest, outside_build};
use gz_project::FileTree;
use gz_task::{Executors, JobCtx};
use rustc_hash::FxHashSet;

use crate::walk::{BuildFile, BuildWalk, FileFrom};

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;

/// What the writing tells the Task Center.
#[derive(Clone, Copy, Debug)]
pub enum Feed<'a> {
    /// A stage of a step (0 Check, 1 Copy, 2 Install); `bytes` and `files` are what it will
    /// count through, 0 when it cannot tell.
    Stage { step: usize, text: &'a str, bytes: u64, files: usize },
    /// One file is written.
    File { rel: &'a RelPath, bytes: u64 },
}

/// A build that did not land.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refusal {
    pub err: String,
    /// The clean build's refusal: the first ten of the files it would delete, and how many.
    pub stale_names: Vec<String>,
    pub stale_count: usize,
}

impl Refusal {
    fn new(err: impl Into<String>) -> Self {
        Self { err: err.into(), ..Self::default() }
    }
}

#[cfg(windows)]
fn is_dir_entry(md: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x10 != 0
}

#[cfg(not(windows))]
fn is_dir_entry(md: &Metadata) -> bool {
    md.is_dir()
}

/// The read-only mark goes first (a file that came off a CD, or the game marked it).
fn clear_readonly(path: &Path, md: &Metadata) {
    let mut perm = md.permissions();
    if perm.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = fs::set_permissions(path, perm);
    }
}

/// Empties a folder and keeps the folder itself. A link goes, what it points at stays - that
/// may be anything, the author's working copy included. Returns the files removed.
pub fn wipe_tree(dir: &Path) -> usize {
    let Ok(listing) = fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in listing.flatten() {
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        let link = md.file_type().is_symlink();
        if link {
            // a junction or a folder link is removed as a folder, a file link as a file: never followed
            if fs::remove_dir(&path).is_err() {
                let _ = fs::remove_file(&path);
            }
        } else if is_dir_entry(&md) {
            removed += wipe_tree(&path);
            let _ = fs::remove_dir(&path);
        } else {
            clear_readonly(&path, &md);
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// A file just deleted lingers while an on-access scanner holds it, and its folder with it.
pub fn remove_tree(dir: &Path) {
    for _ in 0..25 {
        if fs::symlink_metadata(dir).is_err() {
            return;
        }
        wipe_tree(dir);
        if fs::remove_dir(dir).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// An on-access scanner holds a file it has just seen for a moment, and a folder holding such
/// a file does not move.
fn rename_folder(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = io::Error::other("the folder did not move");
    for _ in 0..30 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if matches!(e.raw_os_error(), Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION)) => {
                last = e
            }
            Err(e) => return Err(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(last)
}

/// How much of the target the clean build would throw away: every file there this build does
/// not write (`will`: [`BuildWalk::keys`]), root level included, and every link as one. `names`
/// gets the first ten, for the question that names what goes.
pub fn count_stale(dir: &Path, rel_dir: &str, will: &FxHashSet<String>, names: &mut Vec<String>) -> usize {
    let Ok(listing) = fs::read_dir(dir) else { return 0 };
    let mut entries: Vec<(String, PathBuf)> =
        listing.flatten().filter_map(|e| e.file_name().into_string().ok().map(|n| (n, e.path()))).collect();
    entries.sort_by_key(|(n, _)| n.to_ascii_lowercase());
    let mut n = 0;
    for (name, path) in entries {
        let rel = if rel_dir.is_empty() { name } else { format!("{rel_dir}\\{name}") };
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        let stale = if md.file_type().is_symlink() {
            true
        } else if is_dir_entry(&md) {
            n += count_stale(&path, &rel, will, names);
            false
        } else {
            !will.contains(&rel.to_lowercase())
        };
        if stale {
            n += 1;
            if names.len() < 10 {
                names.push(rel);
            }
        }
    }
    n
}

/// Whether `folder` holds this very file: same size, same write time - or, for bytes made for
/// the build (a small text), the same bytes.
#[must_use]
pub fn same_file_there(f: &BuildFile, folder: &Path) -> bool {
    let there = f.rel.to_fs(folder);
    if let FileFrom::Bytes(bytes) = &f.from {
        return fs::symlink_metadata(&there).is_ok_and(|md| md.is_file() && md.len() == f.size)
            && fs::read(&there).is_ok_and(|b| *b == **bytes);
    }
    let Some(modified) = f.modified else { return false };
    fs::symlink_metadata(there)
        .is_ok_and(|md| md.is_file() && md.len() == f.size && md.modified().ok() == Some(modified))
}

enum Copied {
    Done,
    Cancelled,
    Skipped,
    Failed(String),
}

/// Writes the walked files below `dst`, `strip` taken off the front of each path (the flat
/// overlay writes `gamedata\` into its own root). A file marked in `reuse` is linked from
/// `reuse_from`, where the last build left it. A file that cannot be written fails the build: a
/// module short of a file would go out as a release whose index says it is complete. The files
/// are written in parallel; Stop is checked before each.
#[allow(clippy::too_many_arguments)]
pub fn copy_files(
    files: &[BuildFile],
    root: &Path,
    dst: &Path,
    strip: Option<&RelPath>,
    reuse: &[bool],
    reuse_from: &Path,
    exec: &Executors,
    ctx: &JobCtx,
    feed: &(dyn Fn(Feed<'_>) + Sync),
) -> Result<usize, String> {
    let dst_len = dst.as_os_str().len();
    let mut jobs: Vec<(usize, RelPath)> = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        let out = strip.and_then(|s| f.rel.strip_prefix(s)).unwrap_or_else(|| f.rel.clone());
        if dst_len + 1 + out.as_str().len() >= crate::walk::PATH_LIMIT {
            return Err(format!("the path is too long for the target folder: {}", out.to_backslash()));
        }
        jobs.push((i, out));
    }
    // files come in folder order: every folder once, before the files go in
    let mut folders: Vec<RelPath> = jobs.iter().filter_map(|(_, rel)| rel.parent()).collect();
    folders.sort();
    folders.dedup();
    for folder in &folders {
        let _ = fs::create_dir_all(folder.to_fs(dst));
    }
    let stop = AtomicBool::new(false);
    let results = exec.par_io(jobs, |(i, out)| {
        if stop.load(Ordering::Relaxed) {
            return Copied::Skipped;
        }
        if ctx.cancel.is_cancelled() {
            stop.store(true, Ordering::Relaxed);
            return Copied::Cancelled;
        }
        let Some(f) = files.get(i) else { return Copied::Skipped };
        let to = out.to_fs(dst);
        feed(Feed::File { rel: &out, bytes: f.size });
        if reuse.get(i).copied().unwrap_or(false) && fs::hard_link(out.to_fs(reuse_from), &to).is_ok() {
            return Copied::Done;
        }
        let written = match (&f.from, f.source(root)) {
            (FileFrom::Bytes(bytes), _) => fs::write(&to, bytes),
            (_, Some(from)) => fs::copy(from, &to).map(|_| ()),
            (_, None) => Ok(()),
        };
        match written {
            Ok(()) => Copied::Done,
            Err(e) => {
                stop.store(true, Ordering::Relaxed);
                Copied::Failed(format!(
                    "cannot copy {} (error {})",
                    f.rel.to_backslash(),
                    e.raw_os_error().unwrap_or(0)
                ))
            }
        }
    });
    let mut copied = 0;
    let mut cancelled = false;
    for r in results {
        match r {
            Copied::Done => copied += 1,
            Copied::Failed(why) => return Err(why),
            Copied::Cancelled => cancelled = true,
            Copied::Skipped => {}
        }
    }
    if cancelled || ctx.cancel.is_cancelled() {
        return Err("cancelled".to_owned());
    }
    Ok(copied)
}

/// The module build into `dst` (`<target>\modules\<id>`). Without `clean`, a folder there that
/// holds files this build does not write is refused with them named: it may be somebody else's
/// module that happens to share the id.
pub fn install_build(
    walk: &BuildWalk,
    dst: &Path,
    clean: bool,
    exec: &Executors,
    ctx: &JobCtx,
    feed: &(dyn Fn(Feed<'_>) + Sync),
) -> Result<usize, Refusal> {
    let had = fs::metadata(dst).is_ok_and(|m| m.is_dir());
    if !clean && had {
        let mut names = Vec::new();
        let count = count_stale(dst, "", &walk.keys(), &mut names);
        if count > 0 {
            return Err(Refusal {
                err: format!(
                    "target already holds a module: {count} file(s) there are not part of this build"
                ),
                stale_names: names,
                stale_count: count,
            });
        }
    }
    let (Some(parent), Some(leaf)) = (dst.parent(), dst.file_name()) else {
        return Err(Refusal::new("can't create target folder"));
    };
    let yard = parent.join(".new");
    let staged = yard.join(leaf);
    let mut old_name = leaf.to_os_string();
    old_name.push(".old");
    let previous = yard.join(old_name);
    let _ = fs::create_dir_all(&staged);
    wipe_tree(&staged);
    remove_tree(&previous);
    if !staged.is_dir() {
        return Err(Refusal::new("can't create target folder"));
    }
    let clean_up = || {
        remove_tree(&staged);
        let _ = fs::remove_dir(&yard);
    };

    feed(Feed::Stage { step: 1, text: "Comparing with the last build", bytes: 0, files: 0 });
    let reuse: Vec<bool> = if had {
        exec.par_io(walk.files.iter().collect(), |f| same_file_there(f, dst))
    } else {
        vec![false; walk.files.len()]
    };
    if ctx.cancel.is_cancelled() {
        clean_up();
        return Err(Refusal::new("cancelled"));
    }
    let need: u64 = walk.files.iter().zip(&reuse).filter(|(_, r)| !**r).map(|(f, _)| f.size).sum();
    #[cfg(windows)]
    if let Ok(free) = gz_win::free_space(&yard)
        && free < need
    {
        clean_up();
        return Err(Refusal::new(format!(
            "the build needs {} free on the target's drive and has {}: the new build is written beside the last one before it replaces it",
            crate::job::bytes_text(need),
            crate::job::bytes_text(free)
        )));
    }
    #[cfg(not(windows))]
    let _ = need;

    feed(Feed::Stage { step: 1, text: "Copying files", bytes: walk.bytes, files: walk.files.len() });
    let files = match copy_files(&walk.files, &walk.root, &staged, None, &reuse, dst, exec, ctx, feed) {
        Ok(n) => n,
        Err(why) => {
            clean_up();
            return Err(Refusal::new(why));
        }
    };

    feed(Feed::Stage { step: 2, text: "Putting the module in place", bytes: 0, files: 0 });
    if had && rename_folder(dst, &previous).is_err() {
        clean_up();
        return Err(Refusal::new(format!(
            "the module in {} is in use: close the game and build again. It stays as it was",
            dst.display()
        )));
    }
    if let Err(e) = rename_folder(&staged, dst) {
        if had {
            let _ = rename_folder(&previous, dst);
        }
        clean_up();
        return Err(Refusal::new(format!(
            "can't put the build in place (error {})",
            e.raw_os_error().unwrap_or(0)
        )));
    }
    remove_tree(&previous);
    let _ = fs::remove_dir(&yard);
    Ok(files)
}

/// The flat overlay into `dst` (`<target>\gamedata_<id>`): `gamedata` alone, plus the report of
/// what a plain gamedata folder cannot carry. It IS a JSGME mod, so it keeps its own name.
pub fn export_flat(
    walk: &BuildWalk,
    dst: &Path,
    report: &str,
    exec: &Executors,
    ctx: &JobCtx,
    feed: &(dyn Fn(Feed<'_>) + Sync),
) -> Result<usize, Refusal> {
    let _ = fs::create_dir_all(dst);
    if !dst.is_dir() {
        return Err(Refusal::new("can't create target folder"));
    }
    feed(Feed::Stage { step: 1, text: "Copying files", bytes: walk.bytes, files: walk.files.len() });
    let gamedata = RelPath::new("gamedata").map_err(|e| Refusal::new(e.to_string()))?;
    let files = copy_files(&walk.files, &walk.root, dst, Some(&gamedata), &[], dst, exec, ctx, feed)
        .map_err(Refusal::new)?;
    let _ = fs::write(dst.join("_xms_export_report.txt"), report);
    Ok(files)
}

fn count_under(tree: &FileTree, folder: &str, skip_name: Option<&str>) -> usize {
    let Ok(rel) = RelPath::new(folder) else { return 0 };
    tree.files_under(&rel)
        .iter()
        .filter(|(_, e)| skip_name.is_none_or(|s| !e.name.eq_ignore_ascii_case(s)))
        .count()
}

/// The report a flat export leaves in its folder: XMS-only content is not representable as a
/// plain gamedata overlay. Counted from the file tree.
#[must_use]
pub fn flat_report(tree: &FileTree, m: &Manifest) -> String {
    let c_patch = count_under(tree, "patch", Some("_readme.txt"));
    let c_spawn = count_under(tree, "spawn", None);
    let c_scripts = count_under(tree, "scripts", None);
    let c_levels = count_under(tree, "levels", None);
    let module_files = |ext: &str| {
        tree.files_with_extension(ext).into_iter().filter(|p| !outside_build(&m.exclude, p.as_str())).count()
    };
    // quests are module-only content wherever they sit: the NQ runtime finds them under
    // <game>\modules\<id>, which a flat overlay is not
    let c_quests = module_files("nqasset") + module_files("behasset");
    let characters = module_files("npcasset");

    let mut rep = format!("XMS flat export report - module '{}'\r\n\r\n", m.id);
    if c_patch == 0
        && c_spawn == 0
        && c_scripts == 0
        && c_levels == 0
        && c_quests == 0
        && m.mode.is_empty()
        && m.provides_modes.is_empty()
        && characters == 0
    {
        rep.push_str("fully vanilla-compatible\r\n");
        return rep;
    }
    // the game reads a mode and a character of a module, and a flat overlay is none
    if characters > 0 {
        rep.push_str(&format!("characters: {characters} file(s) - NOT exported (XMS-only)\r\n"));
    }
    if let Some(mode) = m.provides_modes.first() {
        rep.push_str(&format!("declared mode: {} - NOT registered: a flat overlay cannot add it to the new-game screen, export as a module for that\r\n", mode.id));
    }
    for (count, what) in [(c_patch, "patch"), (c_spawn, "spawn"), (c_scripts, "scripts")] {
        if count > 0 {
            rep.push_str(&format!("{what}: {count} file(s) - NOT exported (XMS-only)\r\n"));
        }
    }
    if c_levels > 0 {
        rep.push_str(&format!("levels: {c_levels} overlay file(s) - NOT exported (XMS-only)\r\n"));
    }
    if c_quests > 0 {
        rep.push_str(&format!("graphs: {c_quests} .nqasset/.behasset - NOT usable flat (the NQ runtime only scans <game>\\modules\\<id>)\r\n"));
    }
    if !m.mode.is_empty() {
        rep.push_str(&format!("target mode: {} - engine gating is XMS-only (NOT exported flat)\r\n", m.mode));
    }
    rep
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::walk::WalkScope;

    struct Fixture {
        base: PathBuf,
        rt: gz_task::Runtime,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!("gz-build-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&base).unwrap();
            let rt = gz_task::Runtime::start(gz_task::ExecOptions {
                async_threads: 1,
                cpu_threads: 1,
                io_threads: 2,
            })
            .unwrap();
            Self { base, rt }
        }

        fn write(&self, rel: &str, text: &str) {
            let p = self.base.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }

        /// A job context of a real job that waits until the test is done with it.
        fn ctx(&self) -> (JobCtx, job::Gate) {
            job::ctx(&self.rt)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    /// Hands a test the `JobCtx` of a job that stays open while the test runs.
    mod job {
        use std::sync::mpsc;

        use gz_task::{JobCtx, Jobs};

        pub struct Gate(#[allow(dead_code)] pub mpsc::Sender<()>);

        pub fn ctx(rt: &gz_task::Runtime) -> (JobCtx, Gate) {
            let jobs = Jobs::new(rt.exec());
            let (give, take) = mpsc::channel();
            let (hold, wait) = mpsc::channel::<()>();
            let started = jobs.start("test", None, "test", move |ctx| async move {
                let _ = give.send(ctx);
                // the job ends when the test drops its gate
                let _ = wait_on_thread(move || {
                    let _ = wait.recv();
                })
                .await;
                Ok(serde_json::Value::Null)
            });
            assert!(started.is_ok());
            (take.recv().unwrap(), Gate(hold))
        }

        async fn wait_on_thread(f: impl FnOnce() + Send + 'static) -> Result<(), ()> {
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                f();
                let _ = tx.send(());
            });
            rx.await.map_err(|_| ())
        }
    }

    fn noop(_: Feed<'_>) {}

    #[test]
    fn the_new_build_takes_the_place_of_the_last() {
        let fx = Fixture::new("install");
        let project = fx.base.join("project");
        fx.write("project/mod.ltx", "[module]\nid = od\n");
        fx.write("project/gamedata/a.ltx", "a");
        fx.write("project/gamedata/b.ltx", "b");
        let dst = fx.base.join("game\\modules\\od");
        let exec = fx.rt.exec();
        let (ctx, _gate) = fx.ctx();

        let walk = BuildWalk::scan(&FileTree::scan(&project), &[], WalkScope::Module, &exec);
        assert_eq!(install_build(&walk, &dst, false, &exec, &ctx, &noop), Ok(3));
        assert_eq!(fs::read_to_string(dst.join("gamedata\\a.ltx")).unwrap(), "a");
        assert!(!fx.base.join("game\\modules\\.new").exists());

        // something the project does not have: refused without clean, named, nothing touched
        fs::write(dst.join("stale.txt"), "old").unwrap();
        fs::write(project.join("gamedata\\b.ltx"), "B2").unwrap();
        let walk = BuildWalk::scan(&FileTree::scan(&project), &[], WalkScope::Module, &exec);
        let refused = install_build(&walk, &dst, false, &exec, &ctx, &noop).unwrap_err();
        assert_eq!(refused.stale_count, 1);
        assert_eq!(refused.stale_names, ["stale.txt"]);
        assert!(refused.err.starts_with("target already holds a module: 1 file(s)"));
        assert!(dst.join("stale.txt").exists());

        // clean: the stale file goes, the changed one is copied, the unchanged ones are linked
        assert_eq!(install_build(&walk, &dst, true, &exec, &ctx, &noop), Ok(3));
        assert!(!dst.join("stale.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("gamedata\\b.ltx")).unwrap(), "B2");
        assert_eq!(fs::read_to_string(dst.join("gamedata\\a.ltx")).unwrap(), "a");
        assert!(!fx.base.join("game\\modules\\.new").exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_link_goes_and_what_it_points_at_stays() {
        let fx = Fixture::new("wipe");
        fx.write("keep/keep.txt", "mine");
        fx.write("dir/a/b.txt", "x");
        fx.write("dir/c.txt", "x");
        let linked =
            std::os::windows::fs::symlink_dir(fx.base.join("keep"), fx.base.join("dir\\link")).is_ok();
        let removed = wipe_tree(&fx.base.join("dir"));
        assert_eq!(removed, 2);
        assert!(fx.base.join("dir").is_dir());
        assert_eq!(fs::read_dir(fx.base.join("dir")).unwrap().count(), 0);
        if linked {
            assert_eq!(fs::read_to_string(fx.base.join("keep\\keep.txt")).unwrap(), "mine");
        }
        remove_tree(&fx.base.join("dir"));
        assert!(!fx.base.join("dir").exists());
    }

    #[test]
    fn hard_links_reuse_the_last_build() {
        let fx = Fixture::new("links");
        fx.write("project/x.bin", "payload");
        let project = fx.base.join("project");
        let exec = fx.rt.exec();
        let (ctx, _gate) = fx.ctx();
        let walk = BuildWalk::scan(&FileTree::scan(&project), &[], WalkScope::Module, &exec);
        let dst = fx.base.join("t\\modules\\od");
        install_build(&walk, &dst, true, &exec, &ctx, &noop).unwrap();
        let first = fs::metadata(dst.join("x.bin")).unwrap().modified().unwrap();
        install_build(&walk, &dst, true, &exec, &ctx, &noop).unwrap();
        assert_eq!(fs::metadata(dst.join("x.bin")).unwrap().modified().unwrap(), first);
        assert!(same_file_there(&walk.files[0], &dst));
    }

    #[test]
    fn the_flat_report() {
        let fx = Fixture::new("flat");
        fx.write("p/gamedata/configs/a.ltx", "a");
        let tree = FileTree::scan(&fx.base.join("p"));
        let m = Manifest { id: "od".into(), ..Manifest::default() };
        assert_eq!(
            flat_report(&tree, &m),
            "XMS flat export report - module 'od'\r\n\r\nfully vanilla-compatible\r\n"
        );
        fx.write("p/patch/_readme.txt", "x");
        fx.write("p/patch/a.ltxp", "x");
        fx.write("p/quests/q.nqasset", "x");
        fx.write("p/wip/q.nqasset", "x");
        let tree = FileTree::scan(&fx.base.join("p"));
        let m = Manifest {
            id: "od".into(),
            mode: "night".into(),
            exclude: vec!["wip".into()],
            ..Manifest::default()
        };
        let rep = flat_report(&tree, &m);
        assert!(rep.contains("patch: 1 file(s) - NOT exported (XMS-only)\r\n"));
        assert!(rep.contains("graphs: 1 .nqasset/.behasset"));
        assert!(rep.ends_with("target mode: night - engine gating is XMS-only (NOT exported flat)\r\n"));

        let exec = fx.rt.exec();
        let (ctx, _gate) = fx.ctx();
        let walk = BuildWalk::scan(&tree, &m.exclude, WalkScope::Gamedata, &exec);
        let dst = fx.base.join("out\\gamedata_od");
        assert_eq!(export_flat(&walk, &dst, &rep, &exec, &ctx, &noop), Ok(1));
        assert!(dst.join("configs\\a.ltx").is_file());
        assert!(dst.join("_xms_export_report.txt").is_file());
    }
}
