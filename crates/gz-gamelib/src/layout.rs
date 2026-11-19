//! Where the library of a game lives and what its files are called (`docs/GAME_LIBRARY.md`, 2).

use std::path::{Path, PathBuf};

/// The folder of the editor's scratch that holds the libraries, one per game.
pub const FOLDER: &str = "game_library";

/// A game's key: its root as written, lower case, hashed (FNV-1a) - the same game under the
/// same path is the same library on every start.
#[must_use]
pub fn game_key(root: &Path) -> String {
    let text = root.to_string_lossy().to_lowercase().replace('/', "\\");
    let text = text.trim_end_matches('\\');
    let hash = text
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
}

/// The library of one game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    /// The library of the game at `game_root` under the editor's scratch folder `appdata`.
    #[must_use]
    pub fn of(appdata: &Path, game_root: &Path) -> Self {
        Self { root: appdata.join(FOLDER).join(game_key(game_root)) }
    }

    /// `library.lua`: the state for a reader.
    #[must_use]
    pub fn summary(&self) -> PathBuf {
        self.root.join("library.lua")
    }

    /// `state.txt`: what every description was made from.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.root.join("state.txt")
    }

    /// The lock a sync holds: one process brings a library up to date at a time.
    #[must_use]
    pub fn lock(&self) -> PathBuf {
        self.root.join("sync.lock")
    }

    /// The card of a model: `models\<engine name>.meshasset`. `name` is the model's engine name
    /// (its path under `meshes`, no extension), either slash.
    #[must_use]
    pub fn model(&self, name: &str) -> PathBuf {
        self.root.join("models").join(format!("{}.meshasset", name.replace('\\', "/")))
    }

    /// A catalog of one top folder of a kind: `models\index\<top>.lua`, `textures\<top>.lua`,
    /// `sounds\<top>.lua`. Files that lie at the kind's root are the top folder `_root`.
    #[must_use]
    pub fn catalog(&self, kind: &str, top: &str) -> PathBuf {
        let top = if top.is_empty() { "_root" } else { top };
        match kind {
            "models" => self.root.join("models").join("index").join(format!("{top}.lua")),
            _ => self.root.join(kind).join(format!("{top}.lua")),
        }
    }

    /// A list that is one file: `particles`, `sections`, `materials`, `lightanims`, `sound_envs`.
    #[must_use]
    pub fn list(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.lua"))
    }

    /// The folder of a level: its scene, its parts and its passport.
    #[must_use]
    pub fn level(&self, level: &str) -> PathBuf {
        self.root.join("levels").join(level)
    }

    /// A thumbnail: `thumbs\<kind>\<hash of the name>.png`.
    #[must_use]
    pub fn thumb(&self, kind: &str, name: &str) -> PathBuf {
        let key = name.to_lowercase().replace('/', "\\");
        let hash = key
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        self.root.join("thumbs").join(kind).join(format!("{hash:016x}.png"))
    }
}

/// The top folder of a game path below its kind's folder: `dynamics` of
/// `meshes/dynamics/box/a.ogf` (`under` is `meshes/`), and nothing for a file at the kind's root.
#[must_use]
pub fn top_of<'a>(path: &'a str, under: &str) -> &'a str {
    let rest = path.strip_prefix(under).unwrap_or(path);
    match rest.split_once('/') {
        Some((top, _)) => top,
        None => "",
    }
}

/// A game path as the game names the thing: no kind folder, no extension, backslashes.
#[must_use]
pub fn engine_name(path: &str, under: &str) -> String {
    let rest = path.strip_prefix(under).unwrap_or(path);
    let stem = rest.rsplit_once('.').map_or(rest, |(s, _)| s);
    stem.replace('/', "\\")
}

/// Writes a file whole: to a temporary name beside it, then renamed over it, so a reader never
/// meets half a file. The folders are made.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_places() {
        assert_eq!(game_key(Path::new("D:\\Games\\Dead Air")), game_key(Path::new("d:/games/dead air/")));
        assert_ne!(game_key(Path::new("D:\\Games\\Dead Air")), game_key(Path::new("D:\\Games\\Other")));
        let l = Layout { root: PathBuf::from("lib") };
        assert_eq!(l.model("dynamics\\box\\box_1"), PathBuf::from("lib/models/dynamics/box/box_1.meshasset"));
        assert_eq!(l.catalog("textures", "wood"), PathBuf::from("lib/textures/wood.lua"));
        assert_eq!(l.catalog("models", ""), PathBuf::from("lib/models/index/_root.lua"));
        assert_eq!(top_of("meshes/dynamics/box/a.ogf", "meshes/"), "dynamics");
        assert_eq!(top_of("meshes/a.ogf", "meshes/"), "");
        assert_eq!(engine_name("meshes/dynamics/box/a.ogf", "meshes/"), "dynamics\\box\\a");
        assert_eq!(l.thumb("models", "A/b"), l.thumb("models", "a\\B"));
    }
}
