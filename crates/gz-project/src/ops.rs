//! Copy, move, rename, delete and mkdir for the content browser and its commands.
//!
//! The safety contract: the project folder is the ONLY place anything here writes to, renames
//! inside or deletes from. The linked game and the editor's library are read sources - copying
//! OUT of them is fine, moving is not (a move deletes its source). Every path is normalised by
//! its text first ([`paths::lexical_normalize`]) and only then compared with the project root;
//! links are never followed while walking a tree. A delete is final: no Recycle Bin.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::fsys;
use crate::{asset_kind, paths};
use gz_editor::fsx::WriteGuard;
use gz_vfs::GameFiles;

/// The first entries of each detail list a report keeps; the counters stay exact.
const MAX_DETAILS: usize = 64;

#[derive(Clone, Debug)]
pub struct Failure {
    pub path: String,
    pub reason: String,
}

/// What an operation did, accumulated over every entry of a `;` list.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Files actually copied / moved / deleted.
    pub files: u32,
    /// Folders created / removed.
    pub dirs: u32,
    /// Left alone on purpose (it exists, not recursive, ...).
    pub skipped: u32,
    pub failed: u32,
    pub skips: Vec<Failure>,
    pub failures: Vec<Failure>,
}

impl Report {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.failed == 0
    }

    fn fail(&mut self, path: &str, reason: impl Into<String>) {
        self.failed += 1;
        if self.failures.len() < MAX_DETAILS {
            self.failures.push(Failure { path: path.to_owned(), reason: reason.into() });
        }
    }

    fn skip(&mut self, path: &str, reason: impl Into<String>) {
        self.skipped += 1;
        if self.skips.len() < MAX_DETAILS {
            self.skips.push(Failure { path: path.to_owned(), reason: reason.into() });
        }
    }

    /// The reply's fields after `ok`: `files`, `folders`, `skipped`, `failed`, `errors`, `skips`
    /// and `details_truncated` when the lists no longer tell the whole story.
    #[must_use]
    pub fn fields(&self) -> Map<String, Value> {
        let details = |v: &[Failure]| -> Value {
            v.iter()
                .map(|f| json!({ "path": paths::wire(&f.path), "reason": paths::wire(&f.reason) }))
                .collect()
        };
        let mut m = Map::new();
        m.insert("files".into(), json!(self.files));
        m.insert("folders".into(), json!(self.dirs));
        m.insert("skipped".into(), json!(self.skipped));
        m.insert("failed".into(), json!(self.failed));
        m.insert("errors".into(), details(&self.failures));
        m.insert("skips".into(), details(&self.skips));
        if (self.failures.len() as u64) < u64::from(self.failed)
            || (self.skips.len() as u64) < u64::from(self.skipped)
        {
            m.insert("details_truncated".into(), json!(true));
        }
        m
    }
}

fn win32_reason(what: &str, e: &std::io::Error) -> String {
    fsys::win32_text(what, fsys::win32(e))
}

/// A path for reading: a relative one is taken against the project root, never against the
/// process's working directory.
pub fn resolve_source(root: Option<&str>, path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    let joined = if paths::is_absolute(path) {
        path.to_owned()
    } else {
        let Some(root) = root else { return Err("no project is open".into()) };
        format!("{root}\\{path}")
    };
    paths::lexical_normalize(&joined).ok_or_else(|| "malformed path".to_owned())
}

/// A path PROVEN to sit inside the project root.
pub fn resolve_in_project(root: &str, path: &str) -> Result<String, String> {
    let out = resolve_source(Some(root), path)?;
    let root = paths::lexical_normalize(root)
        .ok_or_else(|| "the project root is not a usable absolute path".to_owned())?;
    if !paths::is_inside(&root, &out) {
        return Err("path leaves the project folder - only the project is writable".into());
    }
    Ok(out)
}

/// Creates every missing level of a folder path.
fn ensure_dir(dir: &str) -> bool {
    if fsys::dir_exists(Path::new(dir)) {
        return true;
    }
    let _ = std::fs::create_dir_all(dir);
    fsys::dir_exists(Path::new(dir))
}

fn ensure_dir_reason(dir: &str) -> Result<(), String> {
    if fsys::dir_exists(Path::new(dir)) {
        return Ok(());
    }
    match std::fs::create_dir_all(dir) {
        Ok(()) if fsys::dir_exists(Path::new(dir)) => Ok(()),
        Ok(()) => Err(fsys::win32_text("cannot create the target folder", 0)),
        Err(e) => Err(win32_reason("cannot create the target folder", &e)),
    }
}

/// The file operations on one project. `game` gives the linked game's index, mounted on first
/// need, for a copy whose source is not on disk.
pub struct FileOps<'a> {
    root: &'a str,
    game: &'a (dyn Fn() -> Option<Arc<GameFiles>> + 'a),
    /// Nothing is written under the linked game's root ([`gz_editor::fsx::WriteGuard`]).
    guard: WriteGuard,
    /// Every folder an operation wrote into or removed from, absolute: what the file tree
    /// re-lists.
    pub touched: Vec<String>,
}

impl<'a> FileOps<'a> {
    #[must_use]
    pub fn new(
        root: &'a str,
        game: &'a (dyn Fn() -> Option<Arc<GameFiles>> + 'a),
        guard: WriteGuard,
    ) -> Self {
        Self { root, game, guard, touched: Vec::new() }
    }

    /// A path this operation writes, renames or deletes: inside the project and, where it really
    /// is, not inside the linked game.
    fn writable(&self, path: &str) -> Result<String, String> {
        let out = resolve_in_project(self.root, path)?;
        self.guard.check(Path::new(&out)).map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn touch(&mut self, dir: &str) {
        self.touched.push(dir.to_owned());
    }

    fn copy_one_file(&mut self, src: &str, dst: &str, overwrite: bool, rep: &mut Report) {
        if fsys::file_exists(Path::new(dst)) {
            if !overwrite {
                rep.skip(dst, "already exists (pass overwrite=1)");
                return;
            }
            // a copy refuses a read-only target, and library content is read-only more often than not
            fsys::clear_readonly(Path::new(dst));
        }
        match std::fs::copy(src, dst) {
            Ok(_) => rep.files += 1,
            Err(e) => rep.fail(src, win32_reason("copy failed", &e)),
        }
    }

    fn copy_tree(
        &mut self,
        src_dir: &str,
        dst_dir: &str,
        recursive: bool,
        overwrite: bool,
        rep: &mut Report,
    ) {
        if !ensure_dir(dst_dir) {
            rep.fail(dst_dir, fsys::win32_text("cannot create the target folder", 0));
            return;
        }
        rep.dirs += 1;
        self.touch(dst_dir);
        let Ok(entries) = fsys::list_folder(Path::new(src_dir)) else { return };
        for e in entries {
            let s = paths::join(src_dir, &e.name);
            let d = paths::join(dst_dir, &e.name);
            if e.is_folder() {
                // a junction is not followed: what it points at was never checked against the root
                if e.link {
                    rep.skip(&s, "reparse point, not followed");
                    continue;
                }
                if !recursive {
                    rep.skip(&s, "sub-folder (pass recursive=1)");
                    continue;
                }
                self.copy_tree(&s, &d, true, overwrite, rep);
            } else {
                self.copy_one_file(&s, &d, overwrite, rep);
            }
        }
    }

    fn delete_tree(&mut self, dir: &str, rep: &mut Report) {
        if let Ok(entries) = fsys::list_folder(Path::new(dir)) {
            for e in entries {
                let s = paths::join(dir, &e.name);
                if e.is_folder() {
                    // removing the link itself is right; going through it would delete files
                    // that live outside the project
                    if e.link {
                        match std::fs::remove_dir(&s) {
                            Ok(()) => rep.dirs += 1,
                            Err(err) => rep.fail(&s, win32_reason("cannot remove the link", &err)),
                        }
                        continue;
                    }
                    self.delete_tree(&s, rep);
                } else {
                    fsys::make_writable(Path::new(&s), &e.meta, e.link);
                    match std::fs::remove_file(&s) {
                        Ok(()) => rep.files += 1,
                        Err(err) => rep.fail(&s, win32_reason("delete failed", &err)),
                    }
                }
            }
        }
        fsys::clear_readonly(Path::new(dir));
        match std::fs::remove_dir(dir) {
            Ok(()) => rep.dirs += 1,
            Err(err) => rep.fail(dir, win32_reason("cannot remove the folder", &err)),
        }
    }

    /// Counts what a whole-tree rename is about to move.
    fn count_tree(dir: &str, files: &mut u32, dirs: &mut u32) {
        *dirs += 1;
        let Ok(entries) = fsys::list_folder(Path::new(dir)) else { return };
        for e in entries {
            if e.is_folder() {
                if e.link {
                    *dirs += 1;
                } else {
                    Self::count_tree(&paths::join(dir, &e.name), files, dirs);
                }
            } else {
                *files += 1;
            }
        }
    }

    /// The last resort for a source that is not on disk: a file inside the linked game's
    /// archives. `false` when there is no such file.
    fn copy_from_game(&mut self, src: &str, dst_dir: &str, overwrite: bool, rep: &mut Report) -> bool {
        let Some(game) = (self.game)() else { return false };
        let Some(item) = game.get(src) else { return false };
        let dst = paths::join(dst_dir, item.name());
        if fsys::file_exists(Path::new(&dst)) && !overwrite {
            rep.skip(&dst, "already exists (pass overwrite=1)");
            return true;
        }
        if let Err(why) = ensure_dir_reason(dst_dir) {
            rep.fail(dst_dir, why);
            return true;
        }
        self.touch(dst_dir);
        let Ok(data) = game.read_ref(item) else {
            rep.fail(src, "cannot read the archived file");
            return true;
        };
        fsys::clear_readonly(Path::new(&dst));
        match std::fs::write(&dst, data) {
            Ok(()) => rep.files += 1,
            Err(e) => rep.fail(&dst, win32_reason("cannot create the target file", &e)),
        }
        true
    }

    /// Copies `src` (anywhere: the project, the library, the game) into the project FOLDER
    /// `dst_dir`; the entry keeps its own name.
    pub fn copy(&mut self, src: &str, dst_dir: &str, recursive: bool, overwrite: bool, rep: &mut Report) {
        let dst_root = match self.writable(dst_dir) {
            Ok(p) => p,
            Err(e) => return rep.fail(dst_dir, e),
        };
        let from = match resolve_source(Some(self.root), src) {
            Ok(p) => p,
            Err(e) => return rep.fail(src, e),
        };
        let from_path = Path::new(&from);
        if !fsys::dir_exists(from_path) && !fsys::file_exists(from_path) {
            // not on disk - it may still be a file inside the game's archives
            if !self.copy_from_game(src, &dst_root, overwrite, rep) {
                rep.fail(src, "source not found");
            }
            return;
        }
        let target = paths::join(&dst_root, paths::leaf_of(&from));
        if fsys::dir_exists(from_path) {
            // a folder copied into its own subtree would feed the walk with what it writes
            if paths::is_inside(&from, &target) {
                return rep.fail(src, "destination is inside the source folder");
            }
            if !recursive {
                return rep.skip(&from, "folder (pass recursive=1)");
            }
            self.copy_tree(&from, &target, true, overwrite, rep);
            return;
        }
        if let Err(why) = ensure_dir_reason(&dst_root) {
            return rep.fail(&dst_root, why);
        }
        if from.eq_ignore_ascii_case(&target) {
            return rep.skip(&from, "source and destination are the same file");
        }
        self.touch(&dst_root);
        self.copy_one_file(&from, &target, overwrite, rep);
    }

    /// Moves `src` inside the project into the project FOLDER `dst_dir`. Both sides must be in
    /// the project.
    pub fn move_entry(
        &mut self,
        src: &str,
        dst_dir: &str,
        recursive: bool,
        overwrite: bool,
        rep: &mut Report,
    ) {
        let dst_root = match self.writable(dst_dir) {
            Ok(p) => p,
            Err(e) => return rep.fail(dst_dir, e),
        };
        let from = match self.writable(src) {
            Ok(p) => p,
            Err(e) => {
                return rep.fail(
                    src,
                    format!("{e} (a move deletes its source, so read-only sources can only be copied)"),
                );
            }
        };
        let from_path = Path::new(&from);
        if !fsys::dir_exists(from_path) && !fsys::file_exists(from_path) {
            return rep.fail(src, "source not found");
        }
        let target = paths::join(&dst_root, paths::leaf_of(&from));
        if from.eq_ignore_ascii_case(&target) {
            return rep.skip(&from, "source and destination are the same");
        }
        if let Some(parent) = from_path.parent().and_then(Path::to_str) {
            self.touch(parent);
        }
        self.touch(&dst_root);

        if fsys::file_exists(from_path) {
            let exists = fsys::file_exists(Path::new(&target));
            if exists && !overwrite {
                return rep.skip(&target, "already exists (pass overwrite=1)");
            }
            if let Err(why) = ensure_dir_reason(&dst_root) {
                return rep.fail(&dst_root, why);
            }
            if exists {
                fsys::clear_readonly(Path::new(&target));
            }
            match std::fs::rename(&from, &target) {
                Ok(()) => rep.files += 1,
                // across volumes: copy, then drop the source
                Err(e) if fsys::win32(&e) == fsys::ERROR_NOT_SAME_DEVICE => {
                    match std::fs::copy(&from, &target) {
                        Ok(_) => {
                            fsys::clear_readonly(from_path);
                            match std::fs::remove_file(&from) {
                                Ok(()) => rep.files += 1,
                                Err(e) => rep.fail(&from, win32_reason("move failed", &e)),
                            }
                        }
                        Err(e) => rep.fail(&from, win32_reason("move failed", &e)),
                    }
                }
                Err(e) => rep.fail(&from, win32_reason("move failed", &e)),
            }
            return;
        }

        if paths::is_inside(&from, &target) {
            return rep.fail(src, "destination is inside the source folder");
        }
        if !recursive {
            return rep.skip(&from, "folder (pass recursive=1)");
        }
        if let Err(why) = ensure_dir_reason(&dst_root) {
            return rep.fail(&dst_root, why);
        }
        // same volume and a free name: a plain rename, counted by a cheap walk first
        let target_path = Path::new(&target);
        if !fsys::dir_exists(target_path) && !fsys::file_exists(target_path) {
            let (mut files, mut dirs) = (0, 0);
            Self::count_tree(&from, &mut files, &mut dirs);
            if std::fs::rename(&from, &target).is_ok() {
                rep.files += files;
                rep.dirs += dirs;
                return;
            }
        }
        // merging into a folder that is there, or across volumes: copy the tree, and drop the
        // source only if every entry made it - a skipped copy would otherwise lose the only one
        let (failed_before, skipped_before) = (rep.failed, rep.skipped);
        self.copy_tree(&from, &target, true, overwrite, rep);
        if rep.failed != failed_before || rep.skipped != skipped_before {
            return rep.fail(&from, "source kept: part of the tree was not copied");
        }
        // the copy counted these files already
        let mut cleanup = Report::default();
        self.delete_tree(&from, &mut cleanup);
        rep.failed += cleanup.failed;
        for f in cleanup.failures {
            if rep.failures.len() < MAX_DETAILS {
                rep.failures.push(f);
            }
        }
    }

    /// Deletes a file or folder of the project for good. Never the root itself.
    pub fn delete(&mut self, path: &str, recursive: bool, rep: &mut Report) {
        let victim = match self.writable(path) {
            Ok(p) => p,
            Err(e) => return rep.fail(path, e),
        };
        if paths::lexical_normalize(self.root).is_some_and(|r| r.eq_ignore_ascii_case(&victim)) {
            return rep.fail(path, "refusing to delete the project root itself");
        }
        let vp = Path::new(&victim);
        if let Some(parent) = vp.parent().and_then(Path::to_str) {
            self.touch(parent);
        }
        if fsys::file_exists(vp) {
            fsys::clear_readonly(vp);
            match std::fs::remove_file(vp) {
                Ok(()) => rep.files += 1,
                Err(e) => rep.fail(&victim, win32_reason("delete failed", &e)),
            }
            return;
        }
        if !fsys::dir_exists(vp) {
            return rep.fail(path, "not found");
        }
        if !recursive {
            // removing a folder succeeds on an empty one only: exactly the contract
            match std::fs::remove_dir(vp) {
                Ok(()) => rep.dirs += 1,
                Err(_) => rep.fail(&victim, "folder is not empty (pass recursive=1)"),
            }
            return;
        }
        self.delete_tree(&victim, rep);
    }

    /// Creates a folder (and its missing parents) inside the project; `Ok` with the folder that
    /// now exists. A folder that is there already is what was asked for.
    pub fn mkdir(&mut self, path: &str) -> Result<String, String> {
        let out = self.writable(path)?;
        let p = Path::new(&out);
        if fsys::file_exists(p) {
            return Err("a file of that name is already there".into());
        }
        if fsys::dir_exists(p) {
            return Ok(out);
        }
        match std::fs::create_dir_all(p) {
            Ok(()) if fsys::dir_exists(p) => {
                if let Some(parent) = p.parent().and_then(Path::to_str) {
                    self.touch(parent);
                }
                Ok(out)
            }
            Ok(()) => Err(fsys::win32_text("cannot create the folder", 0)),
            Err(e) => Err(win32_reason("cannot create the folder", &e)),
        }
    }

    /// Renames an entry of the project in place; `new_name` is a leaf. One of the editor's own
    /// assets keeps its extension: the name is taken as a name, dots and all (`route.v2`), the
    /// same extension typed back is fine and another one of the family is refused. A change of
    /// case alone is a rename. `Ok((from, to))`, absolute; equal when nothing changed.
    pub fn rename(&mut self, path: &str, new_name: &str) -> Result<(String, String), String> {
        let from = self.writable(path)?;
        let fp = Path::new(&from);
        let is_file = fsys::file_exists(fp);
        if !fsys::dir_exists(fp) && !is_file {
            return Err("source not found".into());
        }
        let mut leaf = new_name.trim_matches(' ').to_owned();
        if leaf.is_empty() || leaf == "." || leaf == ".." {
            return Err("the new name is empty".into());
        }
        if leaf.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            return Err("a name cannot contain \\ / : * ? \" < > |".into());
        }
        let old_leaf = paths::leaf_of(&from);
        if is_file && let Some(own) = asset_kind::own_extension(old_leaf) {
            match asset_kind::own_extension(&leaf) {
                Some(typed) if !typed.eq_ignore_ascii_case(own) => {
                    return Err(format!(
                        "a rename cannot turn a {own} into a {typed} - the extension is the kind of the asset"
                    ));
                }
                Some(_) => {}
                None => {
                    if leaf.eq_ignore_ascii_case(own) {
                        return Err("the new name is empty".into());
                    }
                    leaf.push_str(own);
                }
            }
        }
        let parent = from.get(..from.len() - old_leaf.len()).unwrap_or_default();
        let out = format!("{parent}{leaf}");
        if out == from {
            return Ok((from, out));
        }
        // the same path in another case is this very entry, not a name that is taken
        let case_only = out.eq_ignore_ascii_case(&from);
        let op = Path::new(&out);
        if !case_only && (fsys::file_exists(op) || fsys::dir_exists(op)) {
            return Err(format!("'{leaf}' is already there"));
        }
        if let Err(e) = std::fs::rename(&from, &out) {
            return Err(win32_reason("cannot rename", &e));
        }
        if let Some(parent) = fp.parent().and_then(Path::to_str) {
            self.touch(parent);
        }
        Ok((from, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct Temp(PathBuf);
    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gz-project-ops-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
        fn root(&self) -> String {
            self.0.to_str().unwrap().to_owned()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn no_game() -> Option<Arc<GameFiles>> {
        None
    }

    #[test]
    fn paths_stay_in_the_project() {
        assert_eq!(resolve_in_project("D:\\P", "a/b").as_deref(), Ok("D:\\P\\a\\b"));
        assert_eq!(
            resolve_in_project("D:\\P", "..\\Other").unwrap_err(),
            "path leaves the project folder - only the project is writable"
        );
        assert_eq!(
            resolve_in_project("D:\\P", "D:\\P_backup\\x").unwrap_err(),
            "path leaves the project folder - only the project is writable"
        );
        assert_eq!(resolve_in_project("D:\\P", "a\\..\\..\\..").unwrap_err(), "malformed path");
        assert_eq!(resolve_source(None, "rel").unwrap_err(), "no project is open");
        assert_eq!(resolve_source(None, "").unwrap_err(), "empty path");
    }

    #[test]
    fn own_assets_keep_their_extension() {
        let t = Temp::new("rename");
        let root = t.root();
        fs::write(t.0.join("NewQuest.nqasset"), "return {}").unwrap();
        fs::write(t.0.join("taken.nqasset"), "return {}").unwrap();
        let darf = no_game;
        let mut ops = FileOps::new(&root, &darf, WriteGuard::default());

        assert_eq!(
            ops.rename("NewQuest.nqasset", "x.npcasset").unwrap_err(),
            "a rename cannot turn a .nqasset into a .npcasset - the extension is the kind of the asset"
        );
        assert_eq!(ops.rename("NewQuest.nqasset", ".nqasset").unwrap_err(), "the new name is empty");
        assert_eq!(ops.rename("NewQuest.nqasset", "  ").unwrap_err(), "the new name is empty");
        assert_eq!(
            ops.rename("NewQuest.nqasset", "a/b").unwrap_err(),
            "a name cannot contain \\ / : * ? \" < > |"
        );
        assert_eq!(ops.rename("NewQuest.nqasset", "taken").unwrap_err(), "'taken.nqasset' is already there");
        assert_eq!(ops.rename("missing.nqasset", "x").unwrap_err(), "source not found");

        let (_, to) = ops.rename("NewQuest.nqasset", "route.v2").unwrap();
        assert!(to.ends_with("\\route.v2.nqasset"));
        let (_, to) = ops.rename("route.v2.nqasset", "Route.V2.NQASSET").unwrap();
        assert!(to.ends_with("\\Route.V2.NQASSET"));
        // a case-only rename lands on the disk
        let names: Vec<String> =
            fs::read_dir(&t.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(names.contains(&"Route.V2.NQASSET".to_owned()));
    }

    /// A project folder inside the linked game: nothing of it is written.
    #[test]
    fn nothing_is_written_under_the_linked_game() {
        let t = Temp::new("guard");
        fs::create_dir_all(t.0.join("gamedata")).unwrap();
        fs::write(t.0.join("gamedata").join("a.txt"), "a").unwrap();
        let root = t.root();
        let darf = no_game;
        let mut ops = FileOps::new(&root, &darf, WriteGuard::new(&root, Some("od")));
        assert!(ops.mkdir("gamedata/new").unwrap_err().contains("linked game's folder"));
        let mut rep = Report::default();
        ops.delete("gamedata/a.txt", false, &mut rep);
        assert_eq!(rep.failed, 1);
        assert!(t.0.join("gamedata").join("a.txt").is_file());
        // the module's install target is the exception
        assert!(ops.mkdir("modules/od").is_ok());
    }

    #[test]
    fn copy_move_delete() {
        let t = Temp::new("copy");
        let root = t.root();
        fs::create_dir_all(t.0.join("src\\sub")).unwrap();
        fs::write(t.0.join("src\\a.txt"), "a").unwrap();
        fs::write(t.0.join("src\\sub\\b.txt"), "b").unwrap();
        let darf = no_game;
        let mut ops = FileOps::new(&root, &darf, WriteGuard::default());

        let mut rep = Report::default();
        ops.copy("src", "dst", true, false, &mut rep);
        assert!(rep.ok());
        assert_eq!((rep.files, rep.dirs), (2, 2));
        let mut rep = Report::default();
        ops.copy("src\\a.txt", "dst\\src", true, false, &mut rep);
        assert_eq!(rep.skipped, 1);
        let mut rep = Report::default();
        ops.copy("src", "src\\sub", true, false, &mut rep);
        assert_eq!(
            rep.failures.first().map(|f| f.reason.as_str()),
            Some("destination is inside the source folder")
        );
        let mut rep = Report::default();
        ops.copy("nothing.txt", "dst", true, false, &mut rep);
        assert_eq!(rep.failures.first().map(|f| f.reason.as_str()), Some("source not found"));

        let mut rep = Report::default();
        ops.move_entry("dst\\src", "moved", true, false, &mut rep);
        assert!(rep.ok(), "{:?}", rep.failures);
        assert!(t.0.join("moved\\src\\sub\\b.txt").is_file());
        assert!(!t.0.join("dst\\src").exists());
        let mut rep = Report::default();
        ops.move_entry("C:\\Windows\\win.ini", "moved", true, false, &mut rep);
        assert!(rep.failures.first().is_some_and(|f| {
            f.reason.ends_with("(a move deletes its source, so read-only sources can only be copied)")
        }));

        let mut rep = Report::default();
        ops.delete("moved", false, &mut rep);
        assert_eq!(
            rep.failures.first().map(|f| f.reason.as_str()),
            Some("folder is not empty (pass recursive=1)")
        );
        let mut rep = Report::default();
        ops.delete("moved", true, &mut rep);
        assert!(rep.ok());
        assert_eq!((rep.files, rep.dirs), (2, 3));
        let mut rep = Report::default();
        ops.delete(".", true, &mut rep);
        assert_eq!(
            rep.failures.first().map(|f| f.reason.as_str()),
            Some("refusing to delete the project root itself")
        );

        assert!(ops.mkdir("gamedata/meshes/mod").unwrap().ends_with("\\gamedata\\meshes\\mod"));
        assert!(ops.mkdir("gamedata/meshes/mod").is_ok());
        assert_eq!(ops.mkdir("src\\a.txt").unwrap_err(), "a file of that name is already there");
    }
}
