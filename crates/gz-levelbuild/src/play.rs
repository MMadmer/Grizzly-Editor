//! Play's game root (D1): the linked game run on the baked scene without a file of its install
//! changed. The root holds copies of the game's engine and libraries (copies, not hard links: a
//! link would share a file of the install with whatever the root's removal does to it), a junction
//! to its archives, a copy of its loose `gamedata` with the level and `editor.spawn` staged into
//! it, and an `appdata` of its own (the player's settings copied the first time, no save, no
//! module). The game's own QA roots are built the same way.
//!
//! What may be a root is [`crate::play_root::check_root`]'s: it is asked before anything else
//! here, and a root is made only in a missing or empty folder, which then carries the marker
//! first. What Play writes into its root, and the only things it clears there:
//! - the marker (`play_root.txt`) with the programs Play copied - a program it listed and no
//!   longer copies is removed, nothing else of the root's top;
//! - `fsgame.ltx`, the game's, overwritten;
//! - `database`, the junction to the game's archives (made once; unlinked, never followed,
//!   wherever a root is cleared);
//! - `gamedata\`, the game's loose gamedata copied anew every time (the old copy removed, a link
//!   in it unlinked), and the staged level in it;
//! - `appdata\` (logs, saves, screenshots, the player's `user.ltx` and the caches, seeded once and
//!   never cleared), `modules\` (empty), `.dead-air-x64\` (the content files copied, the latch
//!   removed).
//!
//! Which programs are copied: the engine, every library the game's own list of its runtime names
//! (`.dead-air-x64\runtime-files.txt`, kept by its installer and updater; without it every library
//! of the game's folder), and every library of the game's folder any of them imports, ordinary or
//! delay-loaded (read from their import tables, [`crate::pe`]) - so a game build that adds a
//! library to its list, or links one it forgot to list, still starts. Never another executable:
//! the updater, the uninstaller and the tools stay where they are.
//!
//! Why the engine does not run where it is installed, with `-fsltx` naming the root's
//! `fsgame.ltx` (which moves `$fs_root$`, `$app_data_root$`, `$game_data$` and the modules root
//! to the root): the game's content service works in the folder of `xrCore.dll`, whatever the fs
//! config says (`Core.ApplicationPath`, `ContentService.cpp`). Every start's verification rewrites
//! `<game>\.dead-air-x64\content-state.txt` (`verify_worker` -> `ContentState::SaveCache`, a
//! truncating write), and the service refuses to start a level unless `$arch_dir$` is that
//! folder's `database` - so the archives stay a junction of the root, not a path in its
//! `fsgame.ltx`.
//!
//! The editor's C++ Play ran IXRay's `Xr3DA.exe`, which read `level.ai.temp` under `-editor_scene`;
//! Dead Air Refined's engine reads `level.ai`, so the staged level folder names it so.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use gz_project::DeleteContext;

use crate::play_root::{self, MARKER, RootState};

/// What of a game folder is its program: the engines a root is looked into for (the linked
/// game's own first), the extensions of its libraries and the game's list of its runtime.
#[derive(Clone, Copy, Debug)]
pub struct GamePrograms {
    pub engines: &'static [&'static str],
    pub libraries: &'static [&'static str],
    /// Relative to the game's folder: one file name of the runtime per line.
    pub runtime_list: Option<&'static str>,
}

/// Dead Air Refined's.
pub const LINKED_GAME: GamePrograms = GamePrograms {
    engines: &["xrEngine.exe", "Xr3DA.exe"],
    libraries: &["dll"],
    runtime_list: Some(".dead-air-x64\\runtime-files.txt"),
};
/// What the game's content service reads beside the executable; a stale incomplete latch would
/// keep it at the main menu.
const CONTENT_DIR: &str = ".dead-air-x64";
const CONTENT_FILES: [&str; 2] = ["content-manifest.txt", "content-state.txt"];
const CONTENT_LATCH: &str = "content-incomplete.txt";
/// Caches of the player's `appdata` seeded once, so a Play does not compile every shader again.
const CACHES: [&str; 4] = ["shaders_cache", "shaders_cache_grizzly", "cdb_cache", "render-hardware.cache"];

/// A game root ready to run.
#[derive(Clone, Debug)]
pub struct PlayRoot {
    pub root: PathBuf,
    pub exe: PathBuf,
    /// The engine and the libraries copied, by name.
    pub programs: Vec<String>,
    pub program_bytes: u64,
    /// What the root was before: `missing`, `empty` or `play_root`.
    pub was: &'static str,
}

/// What a Play prepares: the linked game, the root, what the checks compare with, the longest
/// path of the game's archives (relative to its gamedata) and the level it stages, if any.
pub struct PlayInput<'a> {
    pub game: &'a Path,
    pub root: &'a Path,
    pub context: &'a DeleteContext,
    pub archived: &'a str,
    /// The level's name and the folder it was built into.
    pub level: Option<(&'a str, &'a Path)>,
}

/// The engine of a game folder.
#[must_use]
pub fn engine_of(game: &Path, programs: &GamePrograms) -> Option<PathBuf> {
    programs.engines.iter().map(|e| game.join(e)).find(|p| p.is_file())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() == y.modified().ok(),
        _ => false,
    }
}

/// `src` copied to `dst` unless the copy there is already of the same size and time.
fn copy_changed(src: &Path, dst: &Path) -> Result<(), String> {
    if same_file(src, dst) {
        return Ok(());
    }
    fs::copy(src, dst).map(|_| ()).map_err(|e| format!("{} could not be copied: {e}", src.display()))
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{} could not be made: {e}", dst.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("{} could not be read: {e}", src.display()))?;
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let to = dst.join(e.file_name());
        if t.is_dir() {
            copy_tree(&e.path(), &to)?;
        } else if t.is_file() {
            fs::copy(e.path(), &to)
                .map_err(|err| format!("{} could not be copied: {err}", e.path().display()))?;
        }
    }
    Ok(())
}

/// A folder removed for good; a link in it goes as a link and is never followed.
fn remove_tree(dir: &Path) {
    let Ok(meta) = fs::symlink_metadata(dir) else { return };
    if meta.file_type().is_symlink() || is_junction(dir) {
        let _ = fs::remove_dir(dir).or_else(|_| fs::remove_file(dir));
        return;
    }
    gz_build::install::remove_tree(dir);
}

/// Whether `p` is a reparse point (a junction, a link): its target is not this tree's.
fn is_junction(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        fs::symlink_metadata(p).is_ok_and(|m| m.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        false
    }
}

/// `root\database` as a junction to the game's archives (a junction needs no privilege).
fn link_archives(game: &Path, root: &Path) -> Result<(), String> {
    let link = root.join("database");
    let target = game.join("database");
    if !target.is_dir() {
        return Ok(());
    }
    if is_junction(&link) {
        return Ok(());
    }
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if made {
        Ok(())
    } else {
        Err(format!("the link to the game's archives could not be made in {}", root.display()))
    }
}

/// The files of the game folder's top, by lower-case name; links left out.
fn top_files(game: &Path) -> Result<BTreeMap<String, String>, String> {
    let entries = fs::read_dir(game).map_err(|e| format!("{} could not be read: {e}", game.display()))?;
    Ok(entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(|n| (n.to_ascii_lowercase(), n.to_owned())))
        .collect())
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    name.rsplit_once('.').is_some_and(|(_, x)| extensions.iter().any(|e| e.eq_ignore_ascii_case(x)))
}

/// `start` and every library of `top` that one of them imports, again and again.
fn close_over(
    mut set: std::collections::BTreeSet<String>,
    top: &BTreeMap<String, String>,
    libraries: &[&str],
    mut imports: impl FnMut(&str) -> Vec<String>,
) -> std::collections::BTreeSet<String> {
    let mut queue: Vec<String> = set.iter().cloned().collect();
    while let Some(n) = queue.pop() {
        let Some(name) = top.get(&n) else { continue };
        for i in imports(name) {
            let l = i.to_ascii_lowercase();
            if top.contains_key(&l) && has_extension(&l, libraries) && set.insert(l.clone()) {
                queue.push(l);
            }
        }
    }
    set
}

/// The programs Play copies from `game`: the engine, the runtime's libraries, what they import.
fn programs_of(game: &Path, programs: &GamePrograms, engine: &Path) -> Result<Vec<String>, String> {
    let top = top_files(game)?;
    let mut set = std::collections::BTreeSet::new();
    if let Some(e) = engine.file_name().and_then(|n| n.to_str()) {
        set.insert(e.to_ascii_lowercase());
    }
    let listed = programs.runtime_list.and_then(|l| fs::read(game.join(l)).ok());
    match listed {
        Some(text) => {
            for line in String::from_utf8_lossy(&text).lines() {
                let n = line.trim().to_ascii_lowercase();
                if play_root::plain_name(&n) && top.contains_key(&n) && has_extension(&n, programs.libraries)
                {
                    set.insert(n);
                }
            }
        }
        None => set.extend(top.keys().filter(|n| has_extension(n, programs.libraries)).cloned()),
    }
    let set = close_over(set, &top, programs.libraries, |name| {
        fs::File::open(game.join(name)).map(|mut f| crate::pe::imports(&mut f)).unwrap_or_default()
    });
    Ok(set.iter().filter_map(|l| top.get(l).cloned()).collect())
}

/// Makes `input.root` a game root of the linked game at `input.game`, ready for
/// [`stage_level`]. Every refusal comes before the disk is changed. Blocking.
pub fn prepare(input: &PlayInput<'_>) -> Result<PlayRoot, String> {
    prepare_with(input, &LINKED_GAME)
}

/// [`prepare`] for a game whose programs are `programs`.
pub fn prepare_with(input: &PlayInput<'_>, programs: &GamePrograms) -> Result<PlayRoot, String> {
    let (game, root) = (input.game, input.root);
    let state = play_root::check_root(input.context, root)?;
    let engine = engine_of(game, programs).ok_or_else(|| {
        format!("the linked game at {} has no {}", game.display(), programs.engines.join(" or "))
    })?;
    let fsgame = game.join("fsgame.ltx");
    if !fsgame.is_file() {
        return Err(format!("the linked game at {} has no fsgame.ltx", game.display()));
    }
    let copied = programs_of(game, programs, &engine)?;
    check_path_lengths(root, &planned_longest(input, &state, &copied))?;

    // from here on the disk changes: the marker first, naming what was and what will be copied
    let old = match &state {
        RootState::Marked { programs } => programs.clone(),
        RootState::Missing | RootState::Empty => Vec::new(),
    };
    fs::create_dir_all(root).map_err(|e| format!("{} could not be made: {e}", root.display()))?;
    let write_marker = |names: &[String]| {
        fs::write(root.join(MARKER), play_root::marker_text(names))
            .map_err(|e| format!("{MARKER} could not be written in {}: {e}", root.display()))
    };
    let mut both = copied.clone();
    both.extend(old.iter().filter(|o| !copied.iter().any(|c| c.eq_ignore_ascii_case(o))).cloned());
    write_marker(&both)?;
    let mut bytes = 0;
    for name in &copied {
        let src = game.join(name);
        copy_changed(&src, &root.join(name))?;
        bytes += fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    }
    // a program Play copied before and does not now (the game dropped it): Play's own, so it goes
    for stale in old.iter().filter(|o| !copied.iter().any(|c| c.eq_ignore_ascii_case(o))) {
        let p = root.join(stale);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
            fs::remove_file(&p).map_err(|e| format!("{} could not be removed: {e}", p.display()))?;
        }
    }
    write_marker(&copied)?;
    fs::copy(&fsgame, root.join("fsgame.ltx")).map_err(|e| format!("fsgame.ltx could not be copied: {e}"))?;
    link_archives(game, root)?;
    // the loose gamedata as the game has it now; the level is staged into it afterwards
    let gamedata = root.join("gamedata");
    remove_tree(&gamedata);
    if game.join("gamedata").is_dir() {
        copy_tree(&game.join("gamedata"), &gamedata)?;
    }
    // an appdata of its own: the player's settings, the caches once, no save
    let appdata = root.join("appdata");
    for d in ["logs", "savedgames", "screenshots"] {
        let _ = fs::create_dir_all(appdata.join(d));
    }
    // the player's settings once: the root's own are the root's from then on
    let user = game.join("appdata").join("user.ltx");
    if user.is_file() && !appdata.join("user.ltx").exists() {
        fs::copy(&user, appdata.join("user.ltx"))
            .map_err(|e| format!("user.ltx could not be copied: {e}"))?;
    }
    for c in CACHES {
        let (src, dst) = (game.join("appdata").join(c), appdata.join(c));
        if dst.exists() {
            continue;
        }
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else if src.is_file() {
            let _ = fs::copy(&src, &dst);
        }
    }
    // no module of the player's: the game root has a modules folder of its own, empty
    let _ = fs::create_dir_all(root.join("modules"));
    let content = root.join(CONTENT_DIR);
    let _ = fs::create_dir_all(&content);
    for f in CONTENT_FILES {
        let src = game.join(CONTENT_DIR).join(f);
        if src.is_file() {
            let _ = fs::copy(&src, content.join(f));
        }
    }
    let _ = fs::remove_file(content.join(CONTENT_LATCH));
    let exe = root.join(engine.file_name().unwrap_or_default());
    Ok(PlayRoot { root: root.to_path_buf(), exe, programs: copied, program_bytes: bytes, was: state.name() })
}

/// The longest file path the game takes: `CLocatorAPI::Register` copies every path it indexes into
/// a `string256`, and a longer one stops the game at its start (a breakpoint in xrCore, before its
/// log opens - what a root inside a project folder, or a deep temp folder, did).
pub const GAME_PATH_LIMIT: usize = 255;

/// The longest path below `dir` (relative to the root, backslashes, after `rel`), links not
/// followed.
fn longest_below(dir: &Path, rel: &str, longest: &mut String) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() { name } else { format!("{rel}\\{name}") };
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() || is_junction(&e.path()) {
            continue;
        }
        if t.is_dir() {
            longest_below(&e.path(), &path, longest);
        } else {
            keep_longer(longest, path);
        }
    }
}

fn keep_longer(longest: &mut String, path: String) {
    if path.chars().count() > longest.chars().count() {
        *longest = path;
    }
}

/// The longest path the root will hold once Play has prepared it and staged its level: what
/// it copies of the game, what it stages, what the root keeps (its appdata), the archives.
fn planned_longest(input: &PlayInput<'_>, state: &RootState, programs: &[String]) -> String {
    let mut longest = String::new();
    longest_below(&input.game.join("gamedata"), "gamedata", &mut longest);
    for c in CACHES {
        let src = input.game.join("appdata").join(c);
        if src.is_dir() {
            longest_below(&src, &format!("appdata\\{c}"), &mut longest);
        } else if src.is_file() {
            keep_longer(&mut longest, format!("appdata\\{c}"));
        }
    }
    if matches!(state, RootState::Marked { .. })
        && let Ok(entries) = fs::read_dir(input.root)
    {
        // what the root keeps: gamedata is copied anew and the archives' link is not followed
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("gamedata") || name.eq_ignore_ascii_case("database") {
                continue;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) && !is_junction(&e.path()) {
                longest_below(&e.path(), &name, &mut longest);
            } else {
                keep_longer(&mut longest, name);
            }
        }
    }
    if let Some((level, dir)) = input.level {
        let rel = format!("gamedata\\levels\\{level}");
        longest_below(dir, &rel, &mut longest);
        keep_longer(&mut longest, format!("{rel}\\level.ai"));
        keep_longer(&mut longest, "gamedata\\spawns\\editor.spawn".to_owned());
    }
    if !input.archived.is_empty() {
        keep_longer(&mut longest, format!("gamedata\\{}", input.archived.replace('/', "\\")));
    }
    for p in programs {
        keep_longer(&mut longest, p.clone());
    }
    for f in CONTENT_FILES {
        keep_longer(&mut longest, format!("{CONTENT_DIR}\\{f}"));
    }
    longest
}

/// Refuses a root the game could not start in: `longest`, the longest path it will hold
/// (relative to it), has to fit [`GAME_PATH_LIMIT`] with the root in front.
fn check_path_lengths(root: &Path, longest: &str) -> Result<(), String> {
    let root_text = root.to_string_lossy();
    let total = root_text.trim_end_matches('\\').chars().count() + 1 + longest.chars().count();
    if total > GAME_PATH_LIMIT {
        return Err(format!(
            "The game root {root_text} is too deep for the game: with {longest} in it a path is {total} characters long, and the game takes {GAME_PATH_LIMIT} at most. Give Play a shorter root."
        ));
    }
    Ok(())
}

/// The built level (`level_dir`, every file of it) into the root's `gamedata\levels\<level>`,
/// `level.ai` among them, and the spawn as `gamedata\spawns\editor.spawn`. Only into a root
/// Play made (its marker). Blocking.
pub fn stage_level(
    root: &Path,
    level: &str,
    level_dir: &Path,
    level_ai: &[u8],
    spawn: &[u8],
) -> Result<(), String> {
    if !fs::symlink_metadata(root.join(MARKER)).is_ok_and(|m| m.is_file()) {
        return Err(format!("{} is not a Play root: it has no {MARKER}.", root.display()));
    }
    if !play_root::plain_name(level) {
        return Err(format!("{level} is not a level name"));
    }
    let dst = root.join("gamedata").join("levels").join(level);
    remove_tree(&dst);
    copy_tree(level_dir, &dst)?;
    fs::write(dst.join("level.ai"), level_ai).map_err(|e| format!("level.ai could not be staged: {e}"))?;
    let spawns = root.join("gamedata").join("spawns");
    fs::create_dir_all(&spawns).map_err(|e| format!("{} could not be made: {e}", spawns.display()))?;
    fs::write(spawns.join("editor.spawn"), spawn)
        .map_err(|e| format!("editor.spawn could not be staged: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // no test writes a file named like a program: the game's programs are harmless names here
    const PROGRAMS: GamePrograms =
        GamePrograms { engines: &["engine.bin"], libraries: &["lib"], runtime_list: Some("runtime.txt") };

    fn game(base: &Path) -> PathBuf {
        let game = base.join("game");
        fs::create_dir_all(game.join("gamedata").join("configs")).unwrap();
        fs::create_dir_all(game.join("database")).unwrap();
        fs::create_dir_all(game.join("appdata").join("savedgames")).unwrap();
        fs::create_dir_all(game.join("modules").join("someone")).unwrap();
        fs::write(game.join("engine.bin"), b"engine").unwrap();
        fs::write(game.join("core.lib"), b"library").unwrap();
        fs::write(game.join("unused.lib"), b"an old library").unwrap();
        fs::write(game.join("updater.bin"), b"another program").unwrap();
        fs::write(game.join("runtime.txt"), b"engine.bin\r\ncore.lib\r\n..\\outside.lib\r\n").unwrap();
        fs::write(game.join("readme.txt"), b"not copied").unwrap();
        fs::write(game.join("fsgame.ltx"), b"$fs_root$").unwrap();
        fs::write(game.join("gamedata").join("configs").join("system.ltx"), b"[x]").unwrap();
        fs::write(game.join("database").join("a.db0"), b"db").unwrap();
        fs::write(game.join("appdata").join("user.ltx"), b"vid_mode 800x600").unwrap();
        fs::write(game.join("appdata").join("savedgames").join("s.scop"), b"save").unwrap();
        game
    }

    fn input<'a>(game: &'a Path, root: &'a Path, cx: &'a DeleteContext) -> PlayInput<'a> {
        PlayInput { game, root, context: cx, archived: "", level: None }
    }

    #[test]
    fn a_play_root_mirrors_the_game_and_changes_nothing_of_it() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.exe, root.join("engine.bin"));
        assert_eq!(p.was, "missing");
        assert_eq!(p.programs, ["core.lib", "engine.bin"], "the runtime's list, not every library");
        assert_eq!(fs::read(root.join("core.lib")).unwrap(), b"library");
        assert!(!root.join("unused.lib").exists() && !root.join("updater.bin").exists());
        assert!(!root.join("readme.txt").exists(), "only the programs are copied");
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(marker.contains("program = core.lib") && marker.contains("program = engine.bin"), "{marker}");
        assert!(root.join("gamedata").join("configs").join("system.ltx").is_file());
        assert!(is_junction(&root.join("database")) && root.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 800x600");
        assert!(!root.join("appdata").join("savedgames").join("s.scop").exists(), "no save of the player's");
        assert!(
            root.join("modules").is_dir() && fs::read_dir(root.join("modules")).unwrap().next().is_none()
        );

        let level = base.path().join("built");
        fs::create_dir_all(&level).unwrap();
        fs::write(level.join("level"), b"geometry").unwrap();
        stage_level(&root, "qa_level", &level, b"ai", b"spawn").unwrap();
        let staged = root.join("gamedata").join("levels").join("qa_level");
        assert_eq!(fs::read(staged.join("level")).unwrap(), b"geometry");
        assert_eq!(fs::read(staged.join("level.ai")).unwrap(), b"ai");
        assert_eq!(fs::read(root.join("gamedata").join("spawns").join("editor.spawn")).unwrap(), b"spawn");

        // again, after the game dropped a library and the author left a file in the root: the
        // dropped library goes (Play copied it), the author's file and the root's settings stay
        fs::write(game.join("runtime.txt"), b"engine.bin\r\n").unwrap();
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        fs::write(root.join("appdata").join("user.ltx"), b"vid_mode 1024x768").unwrap();
        let again = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(again.was, "play_root");
        assert_eq!(again.programs, ["engine.bin"]);
        assert!(!root.join("core.lib").exists() && game.join("core.lib").is_file());
        assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"mine");
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 1024x768");
        assert!(!game.join("gamedata").join("levels").exists());
        assert!(!root.join("gamedata").join("levels").exists(), "a new Play stages its level anew");
        // the archives' link goes as a link: the game's archives stay
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(game.join("fsgame.ltx")).unwrap(), b"$fs_root$");
    }

    #[test]
    fn without_the_games_list_every_library_and_never_another_program() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        fs::remove_file(game.join("runtime.txt")).unwrap();
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.programs, ["core.lib", "engine.bin", "unused.lib"]);
        assert!(!root.join("updater.bin").exists());
    }

    #[test]
    fn a_library_one_of_them_imports_is_copied_though_the_list_forgets_it() {
        let top: BTreeMap<String, String> = ["engine.bin", "core.lib", "render.lib", "tool.bin", "other.lib"]
            .iter()
            .map(|n| (n.to_ascii_lowercase(), (*n).to_owned()))
            .collect();
        let start: BTreeSet<String> = ["engine.bin".to_owned()].into();
        let got = close_over(start, &top, &["lib"], |name| match name {
            "engine.bin" => vec!["CORE.lib".to_owned(), "KERNEL32.sys".to_owned(), "tool.bin".to_owned()],
            "core.lib" => vec!["render.lib".to_owned()],
            _ => Vec::new(),
        });
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["core.lib", "engine.bin", "render.lib"], "a program it names is never copied");
    }

    #[test]
    fn every_refusal_comes_before_the_disk_changes() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        // a folder of the author's: refused, untouched
        let work = base.path().join("work");
        fs::create_dir_all(work.join("gamedata")).unwrap();
        fs::write(work.join("gamedata").join("precious.txt"), b"the author's work").unwrap();
        fs::write(work.join("fsgame.ltx"), b"the folder's own").unwrap();
        let why = prepare_with(&input(&game, &work, &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("is not a Play root: choose an empty folder"), "{why}");
        assert_eq!(fs::read(work.join("gamedata").join("precious.txt")).unwrap(), b"the author's work");
        assert_eq!(fs::read(work.join("fsgame.ltx")).unwrap(), b"the folder's own");
        assert_eq!(fs::read_dir(&work).unwrap().count(), 2);
        // a project: refused, untouched
        fs::write(work.join("mod.ltx"), b"[module]").unwrap();
        fs::remove_file(work.join("fsgame.ltx")).unwrap();
        fs::create_dir_all(work.join("empty")).unwrap();
        let why = prepare_with(&input(&game, &work.join("empty"), &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("it lies inside the project"), "{why}");
        assert_eq!(fs::read_dir(work.join("empty")).unwrap().count(), 0);
        // too deep: refused before the root is made
        let root = base.path().join("deep");
        let room = GAME_PATH_LIMIT - root.to_string_lossy().chars().count() - 1 - "gamedata\\".len();
        let fits = PlayInput { archived: &"a".repeat(room), ..input(&game, &root, &cx) };
        assert!(check_path_lengths(&root, &planned_longest(&fits, &RootState::Missing, &[])).is_ok());
        let deep = "a".repeat(room + 1);
        let refused =
            prepare_with(&PlayInput { archived: &deep, ..input(&game, &root, &cx) }, &PROGRAMS).unwrap_err();
        assert!(
            refused.contains("too deep") && refused.contains(&format!("{}", GAME_PATH_LIMIT + 1)),
            "{refused}"
        );
        assert!(!root.exists(), "nothing was made");
        // a level is staged only into a root Play made
        let level = base.path().join("built");
        fs::create_dir_all(&level).unwrap();
        assert!(
            stage_level(&work, "qa_level", &level, b"ai", b"spawn").unwrap_err().contains("not a Play root")
        );
        assert!(!work.join("gamedata").join("levels").exists());
    }
}
