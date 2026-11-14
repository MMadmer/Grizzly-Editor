//! Deleting a project from the Start page: [`check_delete`] is the ONE set of checks in front of
//! the confirmation, the deletion and MCP alike; [`delete_project`] checks again and deletes.
//!
//! Only a listed folder that holds a project file or `mod.ltx`; never the open project, a link,
//! a drive root, the user's own folders, the editor or a game, never one inside the editor or a
//! game, and never one that holds another project. Inside, a link goes as a link and is never
//! followed, and the manifests go last: a deletion that stops half way leaves a project that can
//! be deleted again. The deletion is final - no Recycle Bin.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::fsys::{self, DirItem};
use crate::machine::{self, MachineFile, ProjectState};
use crate::project::{self, ProjectFile};
use crate::{legacy, list, paths};

const MAX_NESTED: usize = 16;
const MAX_FAILED: usize = 32;

/// One entry that stayed behind.
#[derive(Clone, Debug, Serialize)]
pub struct DeleteFailure {
    pub path: String,
    pub reason: String,
}

/// What the check found, or what the deletion did. The confirmation's numbers come from here.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeleteInfo {
    /// The folder, spelled as the list names it.
    pub path: String,
    pub name: String,
    /// Why it is not deleted from here.
    pub refused: Option<String>,
    /// No folder any more: only the list entry goes.
    pub missing: bool,
    pub files: u32,
    pub folders: u32,
    /// Removed as links, never followed.
    pub links: u32,
    pub bytes: u64,
    /// A `.git` folder: the project's whole history.
    pub git: bool,
    /// `releases\`: packaged releases.
    pub releases: bool,
    pub release_files: u32,
    pub release_bytes: u64,
    /// Projects of their own below it (the first sixteen).
    pub nested: Vec<String>,
    pub nested_total: u32,
    /// What stayed behind, and why (the first thirty-two).
    pub failed: Vec<DeleteFailure>,
    pub failed_total: u32,
    pub removed_from_list: bool,
}

impl DeleteInfo {
    fn refuse(&mut self, why: impl Into<String>) {
        self.refused = Some(why.into());
    }

    fn note_failure(&mut self, path: &str, reason: String) {
        self.failed_total += 1;
        if self.failed.len() < MAX_FAILED {
            self.failed.push(DeleteFailure { path: path.to_owned(), reason });
        }
    }
}

/// A place no deletion may take with it: the folder itself and anything holding it, and with
/// `inside` anything inside it as well (the editor, a game).
#[derive(Clone, Debug)]
pub struct Guard {
    pub path: String,
    pub real: Option<String>,
    pub what: String,
    pub inside: bool,
}

impl Guard {
    /// `None` for a path that is not a full one.
    #[must_use]
    pub fn new(path: &str, what: impl Into<String>, inside: bool) -> Option<Guard> {
        if !paths::is_absolute(path) {
            return None;
        }
        let full = paths::full_path(path)?;
        Some(Guard { real: paths::real_path(&full), path: full, what: what.into(), inside })
    }
}

/// What the checks compare with.
#[derive(Clone, Debug, Default)]
pub struct DeleteContext {
    /// The folders `_projects.ini` names, spelled as it has them.
    pub listed: Vec<String>,
    /// The open project's folder.
    pub open: Option<String>,
    pub guards: Vec<Guard>,
}

/// The guards of this machine: the editor (`exe_dir`, `editor_root`), the user's own folders,
/// Windows, Program Files, Temp, the open project's game, the editor's default game, and what
/// every listed project points at on this machine (its game, where it builds to). Reads the
/// listed projects' files and their state in `home`.
///
/// The user's folders are where the shell keeps them, redirected ones
/// included.
#[must_use]
pub fn system_guards(
    exe_dir: &Path,
    editor_root: &Path,
    home: &Path,
    open_game_root: Option<&str>,
    listed: &[String],
) -> Vec<Guard> {
    let mut out = Vec::new();
    let mut add = |path: &str, what: &str, inside: bool| {
        if let Some(g) = Guard::new(path, what, inside) {
            out.push(g);
        }
    };
    if let Some(p) = exe_dir.to_str() {
        add(p, "the editor", true);
    }
    if let Some(p) = editor_root.to_str() {
        add(p, "the editor", true);
    }
    #[cfg(windows)]
    {
        use gz_win::KnownFolder as K;
        for (folder, what) in [
            (K::Profile, "your user profile"),
            (K::Documents, "your Documents folder"),
            (K::Desktop, "your Desktop"),
            (K::Downloads, "your Downloads folder"),
            (K::LocalAppData, "your AppData folder"),
            (K::RoamingAppData, "your AppData folder"),
            (K::Windows, "the Windows folder"),
            (K::ProgramFiles, "Program Files"),
            (K::ProgramFilesX86, "Program Files (x86)"),
        ] {
            if let Some(p) = gz_win::known_folder(folder)
                && let Some(p) = p.to_str()
            {
                add(p, what, false);
            }
        }
    }
    if let Some(p) = std::env::temp_dir().to_str() {
        add(p, "the Temp folder", false);
    }
    if let Some(game) = open_game_root.filter(|g| !g.is_empty()) {
        add(game, "the game the open project is linked to", true);
    }
    let read = |p: &Path| std::fs::read(p).ok().map(|b| gz_base::text::decode_guess(&b).into_owned());
    if let Some(text) = read(&home.join(machine::MACHINE_FILE)) {
        add(&MachineFile::parse(&text).game_root(), "the editor's default game", true);
    }
    for folder in listed {
        let of = format!(" of the project {}", paths::leaf_of(folder));
        let id = project::project_files(Path::new(folder))
            .first()
            .and_then(|f| read(f))
            .and_then(|t| ProjectFile::parse(&t).ok())
            .map(|f| f.id().to_owned())
            .unwrap_or_default();
        if !id.is_empty()
            && let Some(text) = read(&machine::state_path(home, &id))
        {
            let state = ProjectState::parse(&text);
            add(&state.game_root(), &format!("the linked game{of}"), true);
            add(&state.export_target(), &format!("the export target{of}"), true);
        }
        // a project no editor opened since it had project.ltx
        if let Some(text) = read(&Path::new(folder).join(legacy::FILE)) {
            let old = legacy::Legacy::parse(&text);
            add(&old.game_root, &format!("the linked game{of}"), true);
            add(&old.export_target, &format!("the export target{of}"), true);
        }
    }
    out
}

/// A folder a volume is mounted into is the root of that volume.
fn volume_root(path: &str) -> bool {
    #[cfg(windows)]
    return gz_win::is_volume_root(Path::new(path));
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn is_manifest_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(legacy::FILE)
        || name.eq_ignore_ascii_case("mod.ltx")
        || project::is_project_file_name(name)
}

/// A crashed packaging leaves its stage in `temp\` - a copy of the module, no project.
fn is_release_stage(root: &str, dir: &str) -> bool {
    const STAGE: &str = "\\temp\\release_stage_";
    dir.len() > root.len() + STAGE.len()
        && dir.get(root.len()..root.len() + STAGE.len()).is_some_and(|s| s.eq_ignore_ascii_case(STAGE))
}

struct Scan<'a> {
    info: &'a mut DeleteInfo,
    root: String,
    unreadable: Option<(String, u32)>,
}

/// Counts what the folder holds and finds the projects of their own below it; a link is
/// counted, never entered. `false` when a folder cannot be listed.
fn scan_folder(s: &mut Scan<'_>, dir: &str, in_releases: bool) -> bool {
    let entries = match fsys::list_folder(Path::new(dir)) {
        Ok(e) => e,
        Err(code) => {
            s.unreadable = Some((dir.to_owned(), code));
            return false;
        }
    };
    s.info.folders += 1;
    let root = dir.len() == s.root.len();
    let mut manifest = false;
    for e in &entries {
        if e.link {
            s.info.links += 1;
            continue;
        }
        if e.is_folder() {
            let releases = in_releases || (root && e.name.eq_ignore_ascii_case("releases"));
            if root && e.name.eq_ignore_ascii_case(".git") {
                s.info.git = true;
            }
            if root && releases {
                s.info.releases = true;
            }
            if !scan_folder(s, &paths::join(dir, &e.name), releases) {
                return false;
            }
            continue;
        }
        s.info.files += 1;
        s.info.bytes += e.size;
        if in_releases {
            s.info.release_files += 1;
            s.info.release_bytes += e.size;
        }
        if !root && is_manifest_name(&e.name) {
            manifest = true;
        }
    }
    if manifest && !is_release_stage(&s.root, dir) {
        s.info.nested_total += 1;
        if s.info.nested.len() < MAX_NESTED {
            s.info.nested.push(dir.to_owned());
        }
    }
    true
}

fn find_listed(cx: &DeleteContext, path: &str) -> Option<String> {
    if !paths::is_absolute(path) {
        return None;
    }
    cx.listed.iter().find(|e| paths::same_path(e, path)).cloned()
}

/// Every check, and a count of what the folder holds. Deletes nothing. `info.refused` says why
/// it may not be deleted from here; `info.missing` that only the list entry is left.
#[must_use]
pub fn check_delete(cx: &DeleteContext, path: &str) -> DeleteInfo {
    let path = paths::to_backslash(path);
    let mut info = DeleteInfo { path: path.clone(), ..DeleteInfo::default() };
    if !paths::is_absolute(&path) {
        info.refuse("Name the project by the full path of its folder.");
        return info;
    }
    let listed = find_listed(cx, &path);
    let on_list = listed.is_some();
    info.path = listed.or_else(|| paths::full_path(&path)).unwrap_or(path);
    info.name = paths::leaf_of(&info.path).to_owned();

    // gone already: the list entry is all there is to remove
    let Ok(meta) = std::fs::symlink_metadata(&info.path) else {
        if on_list {
            info.missing = true;
        } else {
            info.refuse("There is no such folder, and it is not in the project list.");
        }
        return info;
    };
    if fsys::attributes(&meta) & fsys::DIRECTORY == 0 {
        info.refuse("It is a file, not a project folder.");
        return info;
    }
    // a link: the folder it points at is deleted - if at all - where it really is
    if fsys::is_link(&meta) {
        match paths::real_path(&info.path) {
            Some(target) => info.refuse(format!(
                "The project folder is a link to {target}. Delete the project where it really is."
            )),
            None => info.refuse(
                "The project folder is a link whose target cannot be reached. Remove the link where it is.",
            ),
        }
        return info;
    }
    if !list::has_manifest(Path::new(&info.path)) {
        info.refuse("It holds no project file and no mod.ltx - it is not a project.");
        return info;
    }
    if !on_list {
        info.refuse("It is not in the project list - only a listed project is deleted from here.");
        return info;
    }

    let full = paths::full_path(&info.path).unwrap_or_else(|| info.path.clone());
    let real = paths::real_path(&info.path);
    if let Some(open) = &cx.open {
        let same_real =
            real.as_ref().is_some_and(|r| paths::real_path(open).is_some_and(|o| o.eq_ignore_ascii_case(r)));
        if paths::same_path(open, &info.path) || same_real {
            info.refuse("It is the open project. Close it first: File > Project > Switch Project...");
            return info;
        }
    }
    if paths::is_drive_root(&full)
        || real.as_deref().is_some_and(paths::is_drive_root)
        || volume_root(&full)
        || real.as_deref().is_some_and(volume_root)
    {
        info.refuse("It is the root of a drive.");
        return info;
    }
    for g in &cx.guards {
        let both = real.as_deref().zip(g.real.as_deref());
        let same =
            g.path.eq_ignore_ascii_case(&full) || both.is_some_and(|(r, gr)| gr.eq_ignore_ascii_case(r));
        let holds =
            paths::same_or_below(&g.path, &full) || both.is_some_and(|(r, gr)| paths::same_or_below(gr, r));
        let within = g.inside
            && (paths::same_or_below(&full, &g.path)
                || both.is_some_and(|(r, gr)| paths::same_or_below(r, gr)));
        if same || holds {
            info.refuse(format!("{} {}: {}.", if same { "It is" } else { "It holds" }, g.what, g.path));
            return info;
        }
        if within {
            info.refuse(format!("It lies inside {}: {}.", g.what, g.path));
            return info;
        }
    }

    let root = info.path.clone();
    let mut scan = Scan { info: &mut info, root: root.clone(), unreadable: None };
    if !scan_folder(&mut scan, &root, false) {
        let (dir, code) = scan.unreadable.unwrap_or_default();
        info.refuse(format!(
            "{}.",
            fsys::win32_text(&format!("A folder inside cannot be read: {dir}"), code)
        ));
        return info;
    }
    if info.nested_total > 0 {
        let first = info.nested.first().cloned().unwrap_or_default();
        let why = if info.nested_total == 1 {
            format!("It holds another project: {first}.")
        } else {
            format!("It holds {} other projects, among them {first}.", info.nested_total)
        };
        info.refuse(why);
    }
    info
}

/// What [`projects_inside`] found below a folder.
#[derive(Clone, Debug, Default)]
pub struct Inside {
    /// The folders that hold a project file or `mod.ltx` (the first sixteen), the folder itself
    /// among them when it holds one.
    pub projects: Vec<String>,
    pub total: u32,
    pub files: u32,
    pub folders: u32,
}

/// The question [`check_delete`] asks of a project's folder - does it hold another project? -
/// asked of any folder that is about to be cleared: every folder below `dir`, and `dir` itself,
/// that holds a manifest. A link is counted, never entered. A folder inside that cannot be listed
/// is an error: it could hide a project.
pub fn projects_inside(dir: &str) -> Result<Inside, String> {
    let dir = paths::to_backslash(dir);
    let mut info = DeleteInfo::default();
    let mut scan = Scan { info: &mut info, root: dir.clone(), unreadable: None };
    if !scan_folder(&mut scan, &dir, false) {
        let (at, code) = scan.unreadable.unwrap_or_default();
        return Err(fsys::win32_text(&format!("A folder inside cannot be read: {at}"), code));
    }
    let mut out = Inside { files: info.files, folders: info.folders, ..Inside::default() };
    if list::has_manifest(Path::new(&dir)) {
        out.projects.push(dir.clone());
        out.total += 1;
    }
    out.total += info.nested_total;
    out.projects.extend(info.nested.into_iter().take(MAX_NESTED - out.projects.len()));
    Ok(out)
}

fn fail_reason(code: u32) -> String {
    let what = match code {
        fsys::ERROR_SHARING_VIOLATION | fsys::ERROR_LOCK_VIOLATION => "in use by another program",
        fsys::ERROR_ACCESS_DENIED => "access denied",
        fsys::ERROR_DIR_NOT_EMPTY => "the folder is not empty",
        _ => "cannot be deleted",
    };
    fsys::win32_text(what, code)
}

/// A scanner or an indexer that opened a file a moment ago lets go of it shortly, and a folder
/// whose last entry is still being let go of reads as not empty.
fn delete_entry(path: &Path, folder: bool) -> Result<(), u32> {
    let mut attempt = 0;
    loop {
        let result = if folder { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
        let code = match result {
            Ok(()) => return Ok(()),
            Err(e) => fsys::win32(&e),
        };
        if code == fsys::ERROR_FILE_NOT_FOUND || code == fsys::ERROR_PATH_NOT_FOUND {
            return Ok(());
        }
        let passing = matches!(
            code,
            fsys::ERROR_SHARING_VIOLATION | fsys::ERROR_LOCK_VIOLATION | fsys::ERROR_ACCESS_DENIED
        ) || (folder && code == fsys::ERROR_DIR_NOT_EMPTY);
        if !passing || attempt >= 5 {
            return Err(code);
        }
        attempt += 1;
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Empties `dir` and removes it. At the root the manifests are handed back and the folder
/// stays: the caller finishes both once everything else is gone.
fn wipe_folder(
    dir: &str,
    dir_meta: Option<&std::fs::Metadata>,
    info: &mut DeleteInfo,
    mut root_manifests: Option<&mut Vec<DirItem>>,
) -> bool {
    let entries = match fsys::list_folder(Path::new(dir)) {
        Ok(e) => e,
        Err(code) => {
            info.note_failure(dir, fsys::win32_text("cannot be read", code));
            return false;
        }
    };
    let mut all = true;
    for e in entries {
        let path = paths::join(dir, &e.name);
        let folder = e.is_folder();
        if let Some(manifests) = root_manifests.as_deref_mut()
            && !folder
            && !e.link
            && is_manifest_name(&e.name)
        {
            manifests.push(e);
            continue;
        }
        if folder && !e.link {
            if !wipe_folder(&path, Some(&e.meta), info, None) {
                all = false;
            }
            continue;
        }
        fsys::make_writable(Path::new(&path), &e.meta, e.link);
        match delete_entry(Path::new(&path), folder) {
            Ok(()) => {
                if e.link {
                    info.links += 1;
                } else {
                    info.files += 1;
                    info.bytes += e.size;
                }
            }
            Err(code) => {
                let reason = if e.link {
                    fsys::win32_text("the link cannot be removed", code)
                } else {
                    fail_reason(code)
                };
                info.note_failure(&path, reason);
                all = false;
            }
        }
    }
    if root_manifests.is_some() {
        return all;
    }
    // what stayed inside is named already, and keeps this folder from going anyway
    if !all {
        return false;
    }
    if let Some(meta) = dir_meta {
        fsys::make_writable(Path::new(dir), meta, false);
    }
    match delete_entry(Path::new(dir), true) {
        Ok(()) => {
            info.folders += 1;
            true
        }
        Err(code) => {
            info.note_failure(dir, fail_reason(code));
            false
        }
    }
}

/// A deletion that refused or left something behind: `info` names it, `message` sums it up.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DeleteFailed {
    pub info: Box<DeleteInfo>,
    pub message: String,
}

/// Checks again and deletes the folder for good. A folder that is gone already is only
/// reported `missing` (the caller takes it off the list, as after a success). The list itself
/// is the caller's.
pub fn delete_project(cx: &DeleteContext, path: &str) -> Result<DeleteInfo, DeleteFailed> {
    let info = check_delete(cx, path);
    if let Some(why) = info.refused.clone() {
        return Err(DeleteFailed { info: Box::new(info), message: why });
    }
    if info.missing {
        return Ok(info);
    }

    // the check counted what is there; from here on the numbers are what went
    let mut done = DeleteInfo {
        path: info.path.clone(),
        name: info.name.clone(),
        git: info.git,
        releases: info.releases,
        release_files: info.release_files,
        release_bytes: info.release_bytes,
        ..DeleteInfo::default()
    };
    let root_meta = std::fs::symlink_metadata(&info.path).ok();
    let mut manifests = Vec::new();
    if wipe_folder(&info.path, root_meta.as_ref(), &mut done, Some(&mut manifests)) {
        for m in &manifests {
            let file = paths::join(&info.path, &m.name);
            fsys::make_writable(Path::new(&file), &m.meta, false);
            match delete_entry(Path::new(&file), false) {
                Ok(()) => {
                    done.files += 1;
                    done.bytes += m.size;
                }
                Err(code) => done.note_failure(&file, fail_reason(code)),
            }
        }
        if done.failed_total == 0 {
            if let Some(meta) = &root_meta {
                fsys::make_writable(Path::new(&info.path), meta, false);
            }
            match delete_entry(Path::new(&info.path), true) {
                Ok(()) => done.folders += 1,
                Err(code) => done.note_failure(&info.path, fail_reason(code)),
            }
        }
    }
    // never a success for a folder that is still there
    if done.failed_total == 0 && std::fs::symlink_metadata(&info.path).is_ok() {
        let p = info.path.clone();
        done.note_failure(&p, "the folder is still there".to_owned());
    }
    if done.failed_total > 0 {
        let message = format!(
            "Not deleted completely: {} {} stayed behind, so the project stays in the list.",
            done.failed_total,
            if done.failed_total == 1 { "entry" } else { "entries" }
        );
        return Err(DeleteFailed { info: Box::new(done), message });
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct Temp(PathBuf);
    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gz-project-delete-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
        fn text(&self, rel: &str) -> String {
            self.path(rel).to_str().unwrap().to_owned()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn project(t: &Temp, rel: &str) -> String {
        fs::create_dir_all(t.path(rel)).unwrap();
        fs::write(t.path(rel).join("mod.ltx"), "[module]\r\nid = x\r\n").unwrap();
        t.text(rel)
    }

    #[test]
    fn refuses_what_is_not_a_listed_project() {
        let t = Temp::new("refuse");
        let p = project(&t, "Alpha");
        let cx = DeleteContext::default();
        assert_eq!(
            check_delete(&cx, "Alpha").refused.as_deref(),
            Some("Name the project by the full path of its folder.")
        );
        assert_eq!(
            check_delete(&cx, &p).refused.as_deref(),
            Some("It is not in the project list - only a listed project is deleted from here.")
        );
        let ghost = t.text("Ghost");
        assert_eq!(
            check_delete(&cx, &ghost).refused.as_deref(),
            Some("There is no such folder, and it is not in the project list.")
        );
        fs::create_dir_all(t.path("Plain")).unwrap();
        let cx = DeleteContext { listed: vec![t.text("Plain")], ..DeleteContext::default() };
        assert_eq!(
            check_delete(&cx, &t.text("Plain")).refused.as_deref(),
            Some("It holds no project file and no mod.ltx - it is not a project.")
        );
        fs::write(t.path("file.txt"), "x").unwrap();
        assert_eq!(
            check_delete(&cx, &t.text("file.txt")).refused.as_deref(),
            Some("It is a file, not a project folder.")
        );
    }

    #[test]
    fn a_missing_listed_folder_only_leaves_the_list() {
        let t = Temp::new("missing");
        let ghost = t.text("Ghost");
        let cx = DeleteContext { listed: vec![ghost.clone()], ..DeleteContext::default() };
        let info = check_delete(&cx, &ghost);
        assert!(info.refused.is_none());
        assert!(info.missing);
        assert!(delete_project(&cx, &ghost).unwrap().missing);
    }

    #[test]
    fn refuses_the_open_project_guards_and_nested_projects() {
        let t = Temp::new("guards");
        let p = project(&t, "Alpha");
        let open =
            DeleteContext { listed: vec![p.clone()], open: Some(p.to_lowercase()), guards: Vec::new() };
        assert!(check_delete(&open, &p).refused.unwrap().starts_with("It is the open project."));

        let inside_game = DeleteContext {
            listed: vec![p.clone()],
            open: None,
            guards: vec![Guard::new(&t.text(""), "a game", true).unwrap()],
        };
        assert!(check_delete(&inside_game, &p).refused.unwrap().starts_with("It lies inside a game: "));

        fs::create_dir_all(t.path("Alpha\\gamedata")).unwrap();
        let holds = DeleteContext {
            listed: vec![p.clone()],
            open: None,
            guards: vec![Guard::new(&t.text("Alpha\\gamedata"), "the Temp folder", false).unwrap()],
        };
        assert!(check_delete(&holds, &p).refused.unwrap().starts_with("It holds the Temp folder: "));

        let nested = project(&t, "Alpha\\sub\\Beta");
        let cx = DeleteContext { listed: vec![p.clone()], ..DeleteContext::default() };
        assert_eq!(check_delete(&cx, &p).refused, Some(format!("It holds another project: {nested}.")));
        // a crashed packaging's stage is the editor's own, not a project
        fs::remove_dir_all(t.path("Alpha\\sub")).unwrap();
        project(&t, "Alpha\\temp\\release_stage_1");
        assert!(check_delete(&cx, &p).refused.is_none());
    }

    #[test]
    fn the_projects_inside_a_folder_about_to_be_cleared_are_found() {
        let t = Temp::new("inside");
        fs::create_dir_all(t.path("gamedata\\configs")).unwrap();
        fs::write(t.path("gamedata\\configs\\system.ltx"), "[x]").unwrap();
        let none = projects_inside(&t.text("gamedata")).unwrap();
        assert!(none.projects.is_empty() && none.files == 1 && none.folders == 2, "{none:?}");
        let nested = project(&t, "gamedata\\mods\\my_mod");
        let found = projects_inside(&t.text("gamedata")).unwrap();
        assert_eq!(found.projects, [nested]);
        fs::write(t.path("gamedata\\project.ltx"), "[project]").unwrap();
        assert_eq!(projects_inside(&t.text("gamedata")).unwrap().total, 2);
    }

    #[test]
    fn counts_and_deletes_with_the_manifests_last() {
        let t = Temp::new("wipe");
        let p = project(&t, "Alpha");
        fs::create_dir_all(t.path("Alpha\\.git\\objects")).unwrap();
        fs::write(t.path("Alpha\\.git\\objects\\ab"), "1234").unwrap();
        let mut ro = fs::metadata(t.path("Alpha\\.git\\objects\\ab")).unwrap().permissions();
        ro.set_readonly(true);
        fs::set_permissions(t.path("Alpha\\.git\\objects\\ab"), ro).unwrap();
        fs::create_dir_all(t.path("Alpha\\releases\\1.0.0")).unwrap();
        fs::write(t.path("Alpha\\releases\\1.0.0\\m.zip"), "zipzip").unwrap();
        fs::write(t.path("Alpha\\project.ltx"), "[project]\r\n").unwrap();

        let cx = DeleteContext { listed: vec![p.clone()], ..DeleteContext::default() };
        let info = check_delete(&cx, &p);
        assert!(info.refused.is_none(), "{:?}", info.refused);
        assert!(info.git);
        assert!(info.releases);
        assert_eq!(info.release_files, 1);
        assert_eq!(info.release_bytes, 6);
        assert_eq!(info.files, 4);
        assert_eq!(info.folders, 5);

        let done = delete_project(&cx, &p).unwrap();
        assert_eq!(done.files, 4);
        assert_eq!(done.folders, 5);
        assert!(!t.path("Alpha").exists());
    }
}
