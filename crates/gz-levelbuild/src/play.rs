//! Play's game root (D1): the linked game run on the baked scene without a file of its install
//! changed. The root holds copies of the game's executables (copies, not hard links: a link would
//! share a file of the install with whatever the root's removal does to it), a junction to its
//! archives, a copy of its loose `gamedata` with the level and
//! `editor.spawn` staged into it, and an `appdata` of its own (the player's settings copied the
//! first time, no save, no module). The game's own QA roots are built the same way.
//!
//! The editor's C++ Play ran IXRay's `Xr3DA.exe`, which read `level.ai.temp` under `-editor_scene`;
//! Dead Air Refined's engine reads `level.ai`, so the staged level folder names it so.

use std::fs;
use std::path::{Path, PathBuf};

/// The engines a game root is looked into for, the linked game's own first.
const ENGINES: [&str; 2] = ["xrEngine.exe", "Xr3DA.exe"];
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
pub fn engine_of(game: &Path) -> Option<PathBuf> {
    ENGINES.iter().map(|e| game.join(e)).find(|p| p.is_file())
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
    let engine = engine_of(game)
        .ok_or_else(|| format!("the linked game at {} has no xrEngine.exe or Xr3DA.exe", game.display()))?;
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
        if e.file_type().is_ok_and(|t| t.is_file()) && matches!(ext.as_deref(), Some("exe" | "dll")) {
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
        fs::write(game.join("xrEngine.exe"), b"exe").unwrap();
        fs::write(game.join("xrCore.dll"), b"dll").unwrap();
        fs::write(game.join("fsgame.ltx"), b"$fs_root$").unwrap();
        fs::write(game.join("gamedata").join("configs").join("system.ltx"), b"[x]").unwrap();
        fs::write(game.join("database").join("a.db0"), b"db").unwrap();
        fs::write(game.join("appdata").join("user.ltx"), b"vid_mode 800x600").unwrap();
        fs::write(game.join("appdata").join("savedgames").join("s.scop"), b"save").unwrap();
        let root = base.path().join("project").join("temp").join("play");
        let p = prepare(&game, &root).unwrap();
        assert_eq!(p.exe, root.join("xrEngine.exe"));
        assert_eq!(fs::read(root.join("xrCore.dll")).unwrap(), b"dll");
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
        prepare(&game, &root).unwrap();
        assert!(!game.join("gamedata").join("levels").exists());
        assert!(!root.join("gamedata").join("levels").exists(), "a new Play stages its level anew");
        // the archives' link goes as a link: the game's archives stay
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(game.join("fsgame.ltx")).unwrap(), b"$fs_root$");
    }
}
