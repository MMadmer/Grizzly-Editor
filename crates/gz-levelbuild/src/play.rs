//! Play's game root (D1): the linked game run on the baked scene without a file of its install
//! changed. The root holds copies of the game's executables (copies, not hard links: a link would
//! share a file of the install with whatever the root's removal does to it), a junction to its
//! archives, a copy of its loose `gamedata` with the level and
//! `editor.spawn` staged into it, and an `appdata` of its own (the player's settings copied the
//! first time, no save, no module). The game's own QA roots are built the same way.
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

use std::fs;
use std::path::{Path, PathBuf};

/// What of a game folder is its program: the engines a root is looked into for (the linked
/// game's own first) and the extensions of the files copied beside them.
#[derive(Clone, Copy, Debug)]
pub struct GamePrograms {
    pub engines: &'static [&'static str],
    pub extensions: &'static [&'static str],
}

/// Dead Air Refined's.
pub const LINKED_GAME: GamePrograms =
    GamePrograms { engines: &["xrEngine.exe", "Xr3DA.exe"], extensions: &["exe", "dll"] };
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

/// Whether `p` is a junction (a mount point reparse point): its target is not this tree's.
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

/// Makes `root` a game root of the linked game at `game`, ready for [`stage_level`]. Blocking.
pub fn prepare(game: &Path, root: &Path) -> Result<PlayRoot, String> {
    prepare_with(game, root, &LINKED_GAME)
}

/// [`prepare`] for a game whose programs are `programs`.
pub fn prepare_with(game: &Path, root: &Path, programs: &GamePrograms) -> Result<PlayRoot, String> {
    let engine = engine_of(game, programs).ok_or_else(|| {
        format!("the linked game at {} has no {}", game.display(), programs.engines.join(" or "))
    })?;
    let fsgame = game.join("fsgame.ltx");
    if !fsgame.is_file() {
        return Err(format!("the linked game at {} has no fsgame.ltx", game.display()));
    }
    fs::create_dir_all(root).map_err(|e| format!("{} could not be made: {e}", root.display()))?;
    // the executables and libraries of the game's folder
    let entries = fs::read_dir(game).map_err(|e| format!("{} could not be read: {e}", game.display()))?;
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
        if e.file_type().is_ok_and(|t| t.is_file())
            && ext.as_deref().is_some_and(|x| programs.extensions.contains(&x))
        {
            copy_changed(&p, &root.join(e.file_name()))?;
        }
    }
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
    Ok(PlayRoot { root: root.to_path_buf(), exe })
}

/// The longest file path the game takes: `CLocatorAPI::Register` copies every path it indexes into
/// a `string256`, and a longer one stops the game at its start (a breakpoint in xrCore, before its
/// log opens - what a root inside a project folder, or a deep temp folder, did).
pub const GAME_PATH_LIMIT: usize = 255;

/// The longest path below `dir` (relative, backslashes), the archives' link not followed.
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
        } else if path.chars().count() > longest.chars().count() {
            *longest = path;
        }
    }
}

/// Refuses a root the game could not start in: every file under it - the staged one and, under
/// `gamedata\`, `archived`, the longest path of the game's archives - has to fit
/// [`GAME_PATH_LIMIT`] with the root in front.
pub fn check_path_lengths(root: &Path, archived: &str) -> Result<(), String> {
    let mut longest = String::new();
    longest_below(root, "", &mut longest);
    let archived = format!("gamedata\\{}", archived.replace('/', "\\"));
    if archived.chars().count() > longest.chars().count() {
        longest = archived;
    }
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
/// `level.ai` among them, and the spawn as `gamedata\spawns\editor.spawn`. Blocking.
pub fn stage_level(
    root: &Path,
    level: &str,
    level_dir: &Path,
    level_ai: &[u8],
    spawn: &[u8],
) -> Result<(), String> {
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

    #[test]
    fn a_play_root_mirrors_the_game_and_changes_nothing_of_it() {
        let base = tempfile::tempdir().unwrap();
        let game = base.path().join("game");
        fs::create_dir_all(game.join("gamedata").join("configs")).unwrap();
        fs::create_dir_all(game.join("database")).unwrap();
        fs::create_dir_all(game.join("appdata").join("savedgames")).unwrap();
        fs::create_dir_all(game.join("modules").join("someone")).unwrap();
        // no test writes a file named like a program: the game's programs are a harmless pair here
        let programs = GamePrograms { engines: &["engine.bin"], extensions: &["bin", "lib"] };
        fs::write(game.join("engine.bin"), b"engine").unwrap();
        fs::write(game.join("core.lib"), b"library").unwrap();
        fs::write(game.join("readme.txt"), b"not copied").unwrap();
        fs::write(game.join("fsgame.ltx"), b"$fs_root$").unwrap();
        fs::write(game.join("gamedata").join("configs").join("system.ltx"), b"[x]").unwrap();
        fs::write(game.join("database").join("a.db0"), b"db").unwrap();
        fs::write(game.join("appdata").join("user.ltx"), b"vid_mode 800x600").unwrap();
        fs::write(game.join("appdata").join("savedgames").join("s.scop"), b"save").unwrap();
        let root = base.path().join("project").join("temp").join("play");
        let p = prepare_with(&game, &root, &programs).unwrap();
        assert_eq!(p.exe, root.join("engine.bin"));
        assert_eq!(fs::read(root.join("core.lib")).unwrap(), b"library");
        assert!(!root.join("readme.txt").exists(), "only the programs are copied");
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
        // again: the root is refreshed, the game's gamedata never gains the staged level
        prepare_with(&game, &root, &programs).unwrap();
        assert!(!game.join("gamedata").join("levels").exists());
        assert!(!root.join("gamedata").join("levels").exists(), "a new Play stages its level anew");
        // every path the game will index fits with the root in front, the archives' too
        let room = GAME_PATH_LIMIT - root.to_string_lossy().chars().count() - 1 - "gamedata\\".len();
        assert!(check_path_lengths(&root, &"a".repeat(room)).is_ok());
        let refused = check_path_lengths(&root, &"a".repeat(room + 1)).unwrap_err();
        assert!(
            refused.contains("too deep") && refused.contains(&format!("{}", GAME_PATH_LIMIT + 1)),
            "{refused}"
        );
        // the archives' link goes as a link: the game's archives stay
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(game.join("fsgame.ltx")).unwrap(), b"$fs_root$");
    }
}
