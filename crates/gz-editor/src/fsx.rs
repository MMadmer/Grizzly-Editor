//! File helpers every area uses. Blocking: call them on the IO pool.

use std::io;
use std::path::{Path, PathBuf};

/// Writes a file so that a reader never sees half of it: the bytes go to a sibling temporary
/// file that then takes the target's place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".gz~");
    let tmp = Path::new(&tmp);
    std::fs::write(tmp, bytes)?;
    match std::fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(e)
        }
    }
}

/// The one check every writer of the editor makes before it writes: nothing goes under the linked
/// game's root except the module's install target (`modules\<id>` and its stage in
/// `modules\.new`) and the flat export's `gamedata_<id>`. A path counts both as written and where
/// it really is (the deepest part of it that exists, resolved), so a link cannot carry a write
/// into the game. No game linked: nothing is refused. Made without touching the disk; a check
/// resolves the paths it compares.
#[derive(Clone, Debug, Default)]
pub struct WriteGuard {
    game: Option<PathBuf>,
    allowed: Vec<PathBuf>,
}

/// A folder as written and as it really is: lower case, backslashes, no trailing one.
#[derive(Clone, Debug)]
struct Spelled {
    written: String,
    real: Option<String>,
}

impl Spelled {
    fn of(path: &Path) -> Self {
        Self { written: spell(&path.to_string_lossy()), real: real_path(path).map(|p| spell(&p)) }
    }

    /// Whether the spelled path `p` is this folder or lies below it, as written or as it is.
    fn holds(&self, p: &str) -> bool {
        let below =
            |root: &str| p.strip_prefix(root).is_some_and(|rest| rest.is_empty() || rest.starts_with('\\'));
        below(&self.written) || self.real.as_deref().is_some_and(below)
    }
}

fn spell(p: &str) -> String {
    let mut s = p.replace('/', "\\").to_lowercase();
    for prefix in ["\\\\?\\unc\\", "\\\\?\\"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = if prefix.contains("unc") { format!("\\\\{rest}") } else { rest.to_owned() };
        }
    }
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

/// Where `path` would really be: its deepest existing part resolved, the rest as written.
fn real_path(path: &Path) -> Option<String> {
    let mut head = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(&head) {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Some(out.to_string_lossy().into_owned());
        }
        tail.push(head.file_name()?.to_owned());
        if !head.pop() {
            return None;
        }
    }
}

/// Why a write was refused: a sentence for the author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteRefused(pub String);

impl std::fmt::Display for WriteRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} lies in the linked game's folder: the editor writes there only a module's install target (modules\\<id>) and a flat export (gamedata_<id>)",
            self.0
        )
    }
}

impl std::error::Error for WriteRefused {}

impl WriteGuard {
    /// The guard for a project linked to `game_root` (empty: none); `module` is the id of the
    /// module the project builds, whose install target and flat export are let through.
    #[must_use]
    pub fn new(game_root: &str, module: Option<&str>) -> Self {
        if game_root.trim().is_empty() {
            return Self::default();
        }
        let root = Path::new(game_root);
        let allowed = module
            .filter(|id| one_name(id))
            .map(|id| {
                let modules = root.join("modules");
                let yard = modules.join(".new");
                vec![
                    modules.join(id),
                    yard.join(id),
                    yard.join(format!("{id}.old")),
                    root.join(format!("gamedata_{id}")),
                ]
            })
            .unwrap_or_default();
        Self { game: Some(root.to_path_buf()), allowed }
    }

    /// `Ok` when the editor may write `path` (a file or a folder it writes below). Blocking: it
    /// resolves where the paths really are.
    pub fn check(&self, path: &Path) -> Result<(), WriteRefused> {
        let Some(game) = &self.game else { return Ok(()) };
        let game = Spelled::of(game);
        let p = Spelled::of(path);
        let mut allowed: Option<Vec<Spelled>> = None;
        // each spelling on its own: a target that is allowed as written but really lies elsewhere
        // in the game (a link) is refused
        for s in std::iter::once(p.written.as_str()).chain(p.real.as_deref()) {
            if game.holds(s)
                && !allowed
                    .get_or_insert_with(|| self.allowed.iter().map(|a| Spelled::of(a)).collect())
                    .iter()
                    .any(|a| a.holds(s))
            {
                return Err(WriteRefused(path.display().to_string()));
            }
        }
        Ok(())
    }
}

/// One file-name component a module id may be (no separator, no `..`).
fn one_name(id: &str) -> bool {
    gz_base::file_name_problem(id).is_none()
}

/// Reads a file that may be absent: `Ok(None)` when it does not exist.
pub fn read_opt(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a text file as UTF-8 (a BOM is dropped); a file that is not UTF-8 is read as cp1251.
pub fn read_text(path: &Path) -> io::Result<String> {
    Ok(gz_base::text::decode_guess(&std::fs::read(path)?).into_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_write_under_the_games_gamedata_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let game = base.path().join("Game");
        std::fs::create_dir_all(game.join("gamedata").join("configs")).unwrap();
        std::fs::create_dir_all(game.join("modules")).unwrap();
        let g = game.to_string_lossy().into_owned();
        let guard = WriteGuard::new(&g, Some("od"));

        // the game's own files, as written in any case and with either separator
        for refused in [
            game.join("gamedata").join("configs").join("system.ltx"),
            game.join("GAMEDATA").join("new").join("x.ltx"),
            game.join("modules").join("other").join("mod.ltx"),
            game.join("modules").join(".new").join("other"),
            game.join("bin").join("xrEngine.exe"),
            game.clone(),
        ] {
            assert!(guard.check(&refused).is_err(), "{}", refused.display());
        }
        let msg = guard.check(&game.join("gamedata").join("x")).unwrap_err().to_string();
        assert!(msg.contains("linked game's folder"), "{msg}");
        let slashes = format!("{}/gamedata/configs/a.ltx", g.replace('\\', "/"));
        assert!(guard.check(Path::new(&slashes)).is_err());

        // the module's install target, its stage, the flat export; anything outside the game
        for allowed in [
            game.join("modules").join("od").join("gamedata").join("a.ltx"),
            game.join("Modules").join("OD"),
            game.join("modules").join(".new").join("od").join("mod.ltx"),
            game.join("modules").join(".new").join("od.old"),
            game.join("gamedata_od").join("gamedata").join("x.ltx"),
            base.path().join("project").join("gamedata").join("x.ltx"),
        ] {
            assert!(guard.check(&allowed).is_ok(), "{}", allowed.display());
        }
        // without a module id only the outside is writable; without a game, everything
        assert!(WriteGuard::new(&g, None).check(&game.join("modules").join("od")).is_err());
        assert!(WriteGuard::new(&g, Some("..")).check(&game.join("modules")).is_err());
        assert!(WriteGuard::new("", None).check(&game.join("gamedata")).is_ok());
    }

    #[test]
    fn a_link_does_not_carry_a_write_into_the_game() {
        let base = tempfile::tempdir().unwrap();
        let game = base.path().join("game");
        std::fs::create_dir_all(game.join("gamedata")).unwrap();
        let outside = base.path().join("project");
        std::fs::create_dir_all(&outside).unwrap();
        let link = outside.join("linked");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(game.join("gamedata"))
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(made, "a junction needs no privilege: mklink /J must work");
        let guard = WriteGuard::new(&game.to_string_lossy(), Some("od"));
        assert!(guard.check(&link.join("x.ltx")).is_err());
        assert!(guard.check(&outside.join("x.ltx")).is_ok());
        std::fs::remove_dir(&link).unwrap();
    }
}
