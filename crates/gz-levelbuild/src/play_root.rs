//! What may be Play's game root: [`check_root`] stands in front of every Play that names or makes
//! one, before a byte is written or removed, and [`crate::play::prepare`] asks it again.
//!
//! The questions are those of a project's deletion (`gz-project`'s `delete.rs`): the same
//! [`Guard`]s ([`gz_project::system_guards`]: the editor, the user's folders, Windows, Program
//! Files, Temp, the linked game, every listed project's game, editor and export target), the
//! same comparison - is, holds, lies inside -, as written and where the path really is. On top
//! of them: the open project and every listed one are places a root may not be, hold or lie in,
//! a folder on the way that holds `project.ltx` or `mod.ltx` is a project nobody listed, and a
//! link or a drive root is never a root.
//!
//! A root is Play's when it carries [`MARKER`]: Play writes it when it makes a root in a missing
//! or empty folder, and a folder that holds anything else and no marker is refused. Play clears
//! only what it writes there (`play.rs`), never the folder: the entries of the root's top its
//! marker lists (`entry = `) and the modules it installed in the root's `modules\` (`module = `,
//! written before the module is).

use std::fs;
use std::path::{Path, PathBuf};

use gz_project::{DeleteContext, Guard, paths};

/// The file that makes a folder Play's: made first when Play makes a root, then a line for every
/// entry of the root's top Play writes, before it writes it - the only entries Play ever removes
/// or overwrites there.
pub const MARKER: &str = "play_root.txt";
const MARKER_HEAD: &str = "; A game root made by Play of the level editor. Play clears only what it writes here: the entries\r\n; below, gamedata\\, the staged level; nothing else in this folder is touched.\r\n";
/// What a copying Play listed for the engine it copied into the root: read, so Play removes
/// those copies, and never written.
const PROGRAM_KEY: &str = "program = ";
const ENTRY_KEY: &str = "entry = ";
/// A module Play installed into the root's `modules\`: the only folders Play removes there.
const MODULE_KEY: &str = "module = ";

/// What a root that passed the checks holds now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootState {
    /// No folder yet: Play makes it.
    Missing,
    /// An empty folder: Play makes its root in it.
    Empty,
    /// Play's own root, with the entries its marker lists, the programs a copying Play put
    /// there and the modules Play installed in its `modules\`.
    Marked { programs: Vec<String>, entries: Vec<String>, modules: Vec<String> },
}

impl RootState {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Empty => "empty",
            Self::Marked { .. } => "play_root",
        }
    }
}

/// What a guard says of a root: it is, holds or (for a guard taken `inside`) lies inside it.
fn against(g: &Guard, full: &str, real: Option<&str>) -> Option<String> {
    let both = real.zip(g.real.as_deref());
    let same = g.path.eq_ignore_ascii_case(full) || both.is_some_and(|(r, gr)| gr.eq_ignore_ascii_case(r));
    let holds =
        paths::same_or_below(&g.path, full) || both.is_some_and(|(r, gr)| paths::same_or_below(gr, r));
    let within = g.inside
        && (paths::same_or_below(full, &g.path) || both.is_some_and(|(r, gr)| paths::same_or_below(r, gr)));
    if same || holds {
        Some(format!("it {} {}: {}", if same { "is" } else { "holds" }, g.what, g.path))
    } else if within {
        Some(format!("it lies inside {}: {}", g.what, g.path))
    } else {
        None
    }
}

/// Where a path really is, for a folder that need not exist yet: the nearest folder that does,
/// resolved, with the rest after it.
fn real_of(full: &str) -> Option<String> {
    let mut head = full.to_owned();
    let mut tail: Vec<String> = Vec::new();
    loop {
        if let Some(r) = paths::real_path(&head) {
            let mut out = r;
            for t in tail.iter().rev() {
                out = paths::join(&out, t);
            }
            return Some(out);
        }
        let (parent, leaf) = head.rsplit_once('\\')?;
        if parent.is_empty() || leaf.is_empty() {
            return None;
        }
        tail.push(leaf.to_owned());
        head = if parent.len() == 2 { format!("{parent}\\") } else { parent.to_owned() };
    }
}

fn volume_root(path: &str) -> bool {
    #[cfg(windows)]
    return gz_win::is_volume_root(Path::new(path));
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// The names a marker lists under `key`: plain names only, never a path.
fn marker_names(text: &str, key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in text.lines().filter_map(|l| l.trim().strip_prefix(key.trim_end())).map(str::trim) {
        if plain_name(n) && !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
            out.push(n.to_owned());
        }
    }
    out
}

/// A marker's line for an entry (`entry = `).
pub(crate) fn marker_line(name: &str) -> String {
    format!("{ENTRY_KEY}{name}\r\n")
}

/// A marker's line for a module Play installs (`module = `).
pub(crate) fn module_line(id: &str) -> String {
    format!("{MODULE_KEY}{id}\r\n")
}

/// A file name of the root's top: no separator, no `..`, no stream.
pub(crate) fn plain_name(n: &str) -> bool {
    !n.is_empty() && n != "." && n != ".." && !n.contains(['\\', '/', ':'])
}

/// The marker's head: what a fresh marker starts with.
pub(crate) fn marker_head() -> &'static str {
    MARKER_HEAD
}

/// The marker's text for `entries` and the `modules` Play installed.
pub(crate) fn marker_text(entries: &[String], modules: &[String]) -> String {
    let mut out = MARKER_HEAD.to_owned();
    for e in entries {
        out.push_str(&marker_line(e));
    }
    for m in modules {
        out.push_str(&module_line(m));
    }
    out
}

/// What a marker lists - programs, other entries -, read again (`stage_level`, `prepare`).
pub(crate) fn read_marker(root: &Path) -> Option<(Vec<String>, Vec<String>)> {
    read_marker_all(root).map(|(p, e, _)| (p, e))
}

/// What a marker lists: programs, entries, modules.
pub(crate) fn read_marker_all(root: &Path) -> Option<(Vec<String>, Vec<String>, Vec<String>)> {
    let marker = root.join(MARKER);
    if !fs::symlink_metadata(&marker).is_ok_and(|m| m.is_file()) {
        return None;
    }
    let text = String::from_utf8_lossy(&fs::read(&marker).ok()?).into_owned();
    Some((marker_names(&text, PROGRAM_KEY), marker_names(&text, ENTRY_KEY), marker_names(&text, MODULE_KEY)))
}

/// The projects inside what Play is about to clear of a marked root (its `gamedata`): any
/// `project.ltx` / `mod.ltx` there refuses the root - the question a project's deletion asks
/// (`gz_project::projects_inside`; a link is not entered). Answers how many entries it walked.
pub(crate) fn nothing_to_lose(root: &Path, entries: &[String]) -> Result<u32, String> {
    let mut walked = 0;
    for e in entries.iter().filter(|e| e.eq_ignore_ascii_case("gamedata")) {
        let dir = root.join(e);
        let Ok(meta) = fs::symlink_metadata(&dir) else { continue };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        let inside = gz_project::projects_inside(&dir.to_string_lossy())
            .map_err(|why| format!("Play cannot clear {}: {why}.", dir.display()))?;
        walked += inside.files + inside.folders;
        if let Some(first) = inside.projects.first() {
            return Err(format!(
                "Play cannot clear {}: it holds {} - {first}. Move the project out of the game root first.",
                dir.display(),
                if inside.total == 1 { "a project".to_owned() } else { format!("{} projects", inside.total) }
            ));
        }
    }
    Ok(walked)
}

/// Every check in front of a Play root, and what it holds. Writes nothing.
pub fn check_root(cx: &DeleteContext, root: &Path) -> Result<RootState, String> {
    let Some(text) = root.to_str() else {
        return Err(format!("Play's game root {} is not a path the game can read.", root.display()));
    };
    let text = paths::to_backslash(text);
    if !paths::is_absolute(&text) {
        return Err(format!("Name Play's game root by a full path, not {text}."));
    }
    let full = paths::full_path(&text).ok_or_else(|| format!("{text} is not a path Play can use."))?;
    let refuse = |why: String| Err(format!("Play cannot use {full} as its game root: {why}."));
    let meta = fs::symlink_metadata(&full).ok();
    if let Some(m) = &meta {
        if m.file_type().is_symlink() {
            return match paths::real_path(&full) {
                Some(target) => refuse(format!("it is a link to {target}; choose a real folder")),
                None => refuse("it is a link whose target cannot be reached".to_owned()),
            };
        }
        if !m.is_dir() {
            return refuse("it is a file".to_owned());
        }
    }
    let real = real_of(&full);
    if paths::is_drive_root(&full)
        || real.as_deref().is_some_and(paths::is_drive_root)
        || volume_root(&full)
        || real.as_deref().is_some_and(volume_root)
    {
        return refuse("it is the root of a drive".to_owned());
    }
    let projects = cx.open.iter().chain(&cx.listed).filter_map(|p| {
        Guard::new(p, format!("the project {}", paths::leaf_of(&paths::to_backslash(p))), true)
    });
    for g in projects.chain(cx.guards.iter().cloned()) {
        if let Some(why) = against(&g, &full, real.as_deref()) {
            return refuse(why);
        }
    }
    // a project nobody listed: a folder on the way that holds a manifest
    for spelled in std::iter::once(full.as_str()).chain(real.as_deref()) {
        for dir in Path::new(spelled).ancestors() {
            if gz_project::list::has_manifest(dir) {
                let dir = dir.display().to_string();
                return if dir.eq_ignore_ascii_case(spelled) {
                    refuse("it is a project folder (it holds project.ltx or mod.ltx)".to_owned())
                } else {
                    refuse(format!("it lies inside the project {dir}"))
                };
            }
        }
    }
    if meta.is_none() {
        return Ok(RootState::Missing);
    }
    let entries = fs::read_dir(&full).map_err(|e| format!("{full} cannot be read: {e}"))?;
    let mut names: Vec<String> =
        entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if names.is_empty() {
        return Ok(RootState::Empty);
    }
    if let Some((programs, entries, modules)) = read_marker_all(Path::new(&full)) {
        nothing_to_lose(Path::new(&full), &entries)?;
        return Ok(RootState::Marked { programs, entries, modules });
    }
    names.sort_by_key(|n| n.to_ascii_lowercase());
    let count = names.len();
    let first = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    Err(format!(
        "{full} is not a Play root: choose an empty folder. It holds {count} {} ({first}{}) and no {MARKER}, so Play leaves it as it is.",
        if count == 1 { "entry" } else { "entries" },
        if count > 3 { ", ..." } else { "" }
    ))
}

/// Where Play makes a project's game root when the caller names none, under a short name of the
/// project's path (an older game takes paths of 255 characters at most, `play::path_limit`): an
/// editor started with a home of its own (`-home`, `GRIZZLY_HOME`) keeps it there, `<home>\play\<id>`, so a run that plays
/// leaves nothing in the user's profile; the author's editor (no home given) in the user's local
/// application data, `GrizzlyPlay\<id>` - outside the editor's folders, which a root may not lie in.
#[must_use]
pub fn default_root(project: &Path, home: Option<&Path>) -> PathBuf {
    let base = match home {
        Some(h) => h.join("play"),
        None => {
            #[cfg(windows)]
            let base = gz_win::known_folder(gz_win::KnownFolder::LocalAppData);
            #[cfg(not(windows))]
            let base: Option<PathBuf> = None;
            base.or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
                .unwrap_or_else(std::env::temp_dir)
                .join("GrizzlyPlay")
        }
    };
    // FNV-1a of the project's path: one folder per project, eight characters long
    let key = project.to_string_lossy().replace('/', "\\").to_lowercase();
    let hash = key.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    base.join(format!("{hash:08x}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn text(p: &Path) -> String {
        p.to_str().unwrap().to_owned()
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut out = walk(dir);
        out.sort();
        out
    }

    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let m = fs::symlink_metadata(&p).unwrap();
            out.push(format!("{}|{}", p.display(), m.len()));
            if m.is_dir() && !m.file_type().is_symlink() {
                out.extend(walk(&p));
            }
        }
        out
    }

    #[test]
    fn a_missing_or_empty_folder_or_a_marked_root_passes() {
        let base = tempfile::tempdir().unwrap();
        let cx = DeleteContext::default();
        let missing = base.path().join("new_root");
        assert_eq!(check_root(&cx, &missing).unwrap(), RootState::Missing);
        fs::create_dir_all(&missing).unwrap();
        assert_eq!(check_root(&cx, &missing).unwrap(), RootState::Empty);
        // the programs a copying Play put there are listed too
        let text = format!("{MARKER_HEAD}program = engine.bin\r\nprogram = ..\\x\r\nentry = gamedata\r\n");
        fs::write(missing.join(MARKER), text).unwrap();
        fs::create_dir_all(missing.join("gamedata")).unwrap();
        // a marker names plain file names only
        assert_eq!(
            check_root(&cx, &missing).unwrap(),
            RootState::Marked {
                programs: vec!["engine.bin".to_owned()],
                entries: vec!["gamedata".to_owned()],
                modules: Vec::new(),
            }
        );
    }

    #[test]
    fn a_marked_root_whose_gamedata_holds_a_project_is_refused_and_left_as_it_is() {
        let base = tempfile::tempdir().unwrap();
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        fs::create_dir_all(root.join("gamedata").join("mods").join("my_mod")).unwrap();
        fs::write(root.join(MARKER), marker_text(&["gamedata".to_owned()], &[])).unwrap();
        fs::write(root.join("gamedata").join("mods").join("my_mod").join("mod.ltx"), b"[module]").unwrap();
        let before = listing(base.path());
        let why = check_root(&cx, &root).unwrap_err();
        assert!(why.contains("it holds a project") && why.contains("my_mod"), "{why}");
        assert_eq!(listing(base.path()), before);
        // without it the root passes, and the walk says what it read
        fs::remove_file(root.join("gamedata").join("mods").join("my_mod").join("mod.ltx")).unwrap();
        assert!(matches!(check_root(&cx, &root).unwrap(), RootState::Marked { .. }));
        assert_eq!(nothing_to_lose(&root, &["gamedata".to_owned()]).unwrap(), 3);
    }

    #[test]
    fn a_folder_with_anything_and_no_marker_is_refused_and_left_as_it_is() {
        let base = tempfile::tempdir().unwrap();
        let cx = DeleteContext::default();
        let root = base.path().join("work");
        fs::create_dir_all(root.join("gamedata")).unwrap();
        fs::write(root.join("gamedata").join("precious.txt"), b"the author's work").unwrap();
        let before = listing(base.path());
        let why = check_root(&cx, &root).unwrap_err();
        assert!(
            why.contains("is not a Play root: choose an empty folder") && why.contains("gamedata"),
            "{why}"
        );
        assert_eq!(listing(base.path()), before);
    }

    #[test]
    fn a_project_or_a_folder_inside_one_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let cx = DeleteContext::default();
        let project = base.path().join("Project");
        fs::create_dir_all(project.join("temp")).unwrap();
        fs::write(project.join("mod.ltx"), b"[module]\r\nid = x\r\n").unwrap();
        let before = listing(base.path());
        let why = check_root(&cx, &project).unwrap_err();
        assert!(why.contains("it is a project folder"), "{why}");
        let why = check_root(&cx, &project.join("temp").join("play")).unwrap_err();
        assert!(why.contains("it lies inside the project"), "{why}");
        // the open project and a listed one, by guard: a root that holds one
        let listed = DeleteContext { listed: vec![text(&project.join("temp"))], ..DeleteContext::default() };
        let why = check_root(&listed, &project).unwrap_err();
        assert!(why.contains("it holds the project temp"), "{why}");
        let open = DeleteContext { open: Some(text(&project)), ..DeleteContext::default() };
        let why = check_root(&open, &project.join("x")).unwrap_err();
        assert!(why.contains("it lies inside the project Project"), "{why}");
        assert_eq!(listing(base.path()), before);
    }

    #[test]
    fn the_editor_a_game_a_drive_a_relative_path_and_a_file_are_refused() {
        let base = tempfile::tempdir().unwrap();
        let game = base.path().join("game");
        let editor = base.path().join("editor");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&editor).unwrap();
        let cx = DeleteContext {
            guards: vec![
                Guard::new(&text(base.path()), "the Temp folder", false).unwrap(),
                Guard::new(&text(&game), "the linked game", true).unwrap(),
                Guard::new(&text(&editor), "the editor", true).unwrap(),
            ],
            ..DeleteContext::default()
        };
        let why = check_root(&cx, &game).unwrap_err();
        assert!(why.contains("it is the linked game"), "{why}");
        let why = check_root(&cx, &game.join("play")).unwrap_err();
        assert!(why.contains("it lies inside the linked game"), "{why}");
        let why = check_root(&cx, &editor.join("_appdata_").join("play")).unwrap_err();
        assert!(why.contains("it lies inside the editor"), "{why}");
        let why = check_root(&cx, base.path()).unwrap_err();
        assert!(why.contains("it is the Temp folder"), "{why}");
        // below a guard that is not taken inside (Temp), a fresh folder is fine
        assert_eq!(check_root(&cx, &base.path().join("fresh")).unwrap(), RootState::Missing);
        let why = check_root(&DeleteContext::default(), Path::new("C:\\")).unwrap_err();
        assert!(why.contains("the root of a drive"), "{why}");
        let why = check_root(&DeleteContext::default(), Path::new("relative\\root")).unwrap_err();
        assert!(why.contains("by a full path"), "{why}");
        fs::write(base.path().join("file.txt"), b"x").unwrap();
        let why = check_root(&DeleteContext::default(), &base.path().join("file.txt")).unwrap_err();
        assert!(why.contains("it is a file"), "{why}");
    }

    #[cfg(windows)]
    #[test]
    fn a_link_is_refused_and_a_guard_is_met_where_the_path_really_is() {
        let base = tempfile::tempdir().unwrap();
        let game = base.path().join("game");
        fs::create_dir_all(&game).unwrap();
        let link = base.path().join("link");
        let made = crate::process::console("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&game)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        let cx = DeleteContext {
            guards: vec![Guard::new(&text(&game), "the linked game", true).unwrap()],
            ..DeleteContext::default()
        };
        let why = check_root(&cx, &link).unwrap_err();
        assert!(why.contains("it is a link to"), "{why}");
        // a folder below the link is inside the game where it really is
        let why = check_root(&cx, &link.join("play")).unwrap_err();
        assert!(why.contains("it lies inside the linked game"), "{why}");
        fs::remove_dir(&link).unwrap();
        assert!(game.is_dir());
    }

    #[test]
    fn the_default_root_is_short_and_one_per_project() {
        let a = default_root(Path::new("D:\\XFinedProjects\\Overnight Delivery"), None);
        let b = default_root(Path::new("d:/xfinedprojects/overnight delivery"), None);
        let c = default_root(Path::new("D:\\XFinedProjects\\Test"), None);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.file_name().unwrap().len(), 8);
        assert!(a.parent().unwrap().ends_with("GrizzlyPlay"));
    }

    #[test]
    fn an_editor_with_a_home_of_its_own_plays_under_it() {
        let home = Path::new("C:\\Temp\\grizzly_home_x");
        let project = Path::new("D:\\XFinedProjects\\Test");
        let r = default_root(project, Some(home));
        assert_eq!(r.parent().unwrap(), home.join("play"));
        assert_eq!(r.file_name(), default_root(project, None).file_name());
    }
}
