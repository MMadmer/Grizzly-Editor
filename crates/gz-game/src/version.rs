//! The linked game's version: the file version of its `xrEngine.exe`, `major.minor.build`.
//!
//! The version resource holds a `VS_FIXEDFILEINFO`: signature `0xFEEF04BD`, structure version
//! `0x00010000`, then `dwFileVersionMS` and `dwFileVersionLS`. The first such block of the file
//! is the one the version query answers.

const SIGNATURE: [u8; 4] = 0xFEEF_04BDu32.to_le_bytes();
const STRUCT_VERSION: u32 = 0x0001_0000;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// `"1.4.2"` from the bytes of an executable; `None` without a version resource.
#[must_use]
pub fn exe_version(exe: &[u8]) -> Option<String> {
    let mut from = 0;
    while let Some(i) = exe.get(from..)?.windows(4).position(|w| w == SIGNATURE) {
        let at = from + i;
        if u32_at(exe, at + 4) == Some(STRUCT_VERSION) {
            let ms = u32_at(exe, at + 8)?;
            let ls = u32_at(exe, at + 12)?;
            return Some(format!("{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16));
        }
        from = at + 4;
    }
    None
}

/// The version resource's `EngineAbilities` string (UTF-16) of a program of the game: the words a
/// game build says its engine can do, space-separated. The engine carries it, and so does every
/// library that implements a word (Dead Air Refined: `xrEngine.dll`, `xrCore.dll`, `xrGame.dll`);
/// a word holds only when all of them state it (`gz-levelbuild`'s `play::ability_of`), so a
/// library of an older build makes a mixed installation one that cannot. Dead Air Refined's words
/// are written down in its `ProductVersion.h`:
/// `fsltx-in-place` (on another fs config it writes nothing into its own folder and takes the
/// archives from that config's `$arch_dir$`), `long-paths` (long-path aware; its paths take
/// [`LONG_PATH_LIMIT`] characters where Windows long paths are on). An engine without the
/// string says nothing: an empty list.
#[must_use]
pub fn engine_abilities(exe: &[u8]) -> Vec<String> {
    let key: Vec<u8> = "EngineAbilities\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let Some(at) = exe.windows(key.len()).position(|w| w == key.as_slice()) else {
        return Vec::new();
    };
    // the key, its padding to a 32-bit boundary (zeros), then the value up to its terminator
    let units: Vec<u16> =
        exe[at + key.len()..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let value: Vec<u16> = units.iter().copied().skip_while(|&u| u == 0).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&value).split_whitespace().map(str::to_owned).collect()
}

/// What a `long-paths` engine takes: its path buffers, `string_path` (`2 * MAX_PATH` bytes), less
/// the terminator.
pub const LONG_PATH_LIMIT: usize = 519;

/// The engine's words the editor asks for, as Dead Air Refined's `ProductVersion.h` writes them.
pub mod words {
    /// Started on another fs config, it writes nothing into its own folder and takes the archives
    /// from that config's `$arch_dir$`.
    pub const IN_PLACE: &str = "fsltx-in-place";
    /// Long-path aware: its paths take [`super::LONG_PATH_LIMIT`] characters.
    pub const LONG_PATHS: &str = "long-paths";
    /// It loads a module's pieces of a level beside the level (`levels\<level>\pieces`).
    pub const LEVEL_PIECES: &str = "level-pieces";
    /// It cuts a level's static geometry by a module's `levels\<level>\overlay.cut`.
    pub const LEVEL_CUTS: &str = "level-cuts";
    /// It moves and closes a level's AI nodes by a module's `levels\<level>\overlay.aiedit`.
    pub const AI_EDITS: &str = "ai-edits";
    /// It reads a module's media descriptions (`*.texasset`, `*.sndasset`, `*.meshasset` under
    /// `gamedata\textures`, `sounds`, `meshes`) over the data beside them.
    pub const ASSET_DESCRIPTIONS: &str = "asset-descriptions";
    /// Every word the editor knows, in the order above.
    pub const ALL: [&str; 6] = [IN_PLACE, LONG_PATHS, LEVEL_PIECES, LEVEL_CUTS, AI_EDITS, ASSET_DESCRIPTIONS];
}

/// What of a game folder is its program: the engines a folder is looked into for (the linked
/// game's own first) and the libraries that state the engine's words.
#[derive(Clone, Copy, Debug)]
pub struct GamePrograms {
    pub engines: &'static [&'static str],
    /// The libraries beside the engine that implement its words, each stating them in its own
    /// version resource: a word counts only when the engine and every one of them state it.
    pub stating: &'static [&'static str],
}

/// Dead Air Refined's.
pub const LINKED_GAME: GamePrograms = GamePrograms {
    engines: &["xrEngine.exe", "Xr3DA.exe"],
    stating: &["xrEngine.dll", "xrCore.dll", "xrGame.dll"],
};

/// The words of a program's version resource; one that cannot be read says nothing. Blocking.
#[must_use]
pub fn words_of(program: &std::path::Path) -> Vec<String> {
    std::fs::read(program).map(|b| engine_abilities(&b)).unwrap_or_default()
}

/// Whether the game in `game` states `word`: its engine and every library that implements its
/// words say it (a library of an older build beside a newer engine does not). Reads the version
/// resources. Blocking.
#[must_use]
pub fn game_states(game: &std::path::Path, programs: &GamePrograms, word: &str) -> bool {
    let says = |p: &std::path::Path| words_of(p).iter().any(|w| w == word);
    programs.engines.iter().map(|e| game.join(e)).find(|p| p.is_file()).is_some_and(|exe| says(&exe))
        && programs.stating.iter().all(|l| says(&game.join(l)))
}

/// The version of the game in `game`: its engine's file version, `None` without an engine or a
/// version resource. Blocking.
#[must_use]
pub fn game_version(game: &std::path::Path, programs: &GamePrograms) -> Option<String> {
    let exe = programs.engines.iter().map(|e| game.join(e)).find(|p| p.is_file())?;
    std::fs::read(exe).ok().and_then(|b| exe_version(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn engine_abilities_from_the_version_strings() {
        // program-files: in memory
        let mut exe = b"MZ....".to_vec();
        exe.extend(utf16("ProductVersion\0"));
        exe.extend(utf16("1.6.4\0"));
        exe.extend(utf16("EngineAbilities\0"));
        exe.extend([0, 0]);
        exe.extend(utf16("fsltx-in-place long-paths\0"));
        exe.extend(utf16("VarFileInfo\0"));
        assert_eq!(engine_abilities(&exe), ["fsltx-in-place", "long-paths"]);
        // program-files: in memory
        assert!(engine_abilities(b"MZ an engine that says nothing").is_empty());
    }

    #[test]
    fn a_word_holds_when_the_engine_and_every_library_state_it() {
        let dir = std::env::temp_dir().join(format!("gz-game-words-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // no test writes a file named like a program: harmless names stand for the game's
        const P: GamePrograms = GamePrograms { engines: &["engine.bin"], stating: &["core.lib"] };
        let saying = |words: &str| {
            let mut b = utf16("EngineAbilities\0");
            b.extend([0, 0]);
            b.extend(utf16(&format!("{words}\0")));
            b
        };
        let mut engine = b"xx".to_vec();
        engine.extend_from_slice(&SIGNATURE);
        engine.extend_from_slice(&STRUCT_VERSION.to_le_bytes());
        engine.extend_from_slice(&((1u32 << 16) | 7).to_le_bytes());
        engine.extend_from_slice(&(3u32 << 16).to_le_bytes());
        engine.extend(saying("level-pieces asset-descriptions level-cuts"));
        std::fs::write(dir.join("engine.bin"), &engine).unwrap();
        std::fs::write(dir.join("core.lib"), saying("asset-descriptions level-pieces")).unwrap();
        assert!(game_states(&dir, &P, words::ASSET_DESCRIPTIONS));
        assert!(game_states(&dir, &P, words::LEVEL_PIECES));
        // the library does not say it: a mixed installation cannot
        assert!(!game_states(&dir, &P, words::LEVEL_CUTS));
        assert!(!game_states(&dir, &P, words::AI_EDITS));
        assert_eq!(game_version(&dir, &P).as_deref(), Some("1.7.3"));
        assert!(!game_states(&dir.join("none"), &P, words::ASSET_DESCRIPTIONS));
        assert_eq!(words::ALL.len(), 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fixed_file_info() {
        // program-files: in memory
        let mut exe = b"MZ....VS_VERSION_INFO".to_vec();
        // a stray signature without the structure version is no version block
        exe.extend_from_slice(&SIGNATURE);
        exe.extend_from_slice(&7u32.to_le_bytes());
        exe.extend_from_slice(&SIGNATURE);
        exe.extend_from_slice(&STRUCT_VERSION.to_le_bytes());
        exe.extend_from_slice(&((1u32 << 16) | 4).to_le_bytes());
        exe.extend_from_slice(&((2u32 << 16) | 77).to_le_bytes());
        assert_eq!(exe_version(&exe).as_deref(), Some("1.4.2"));
        // program-files: in memory
        assert_eq!(exe_version(b"MZ no resources"), None);
        assert_eq!(exe_version(&exe[..exe.len() - 2]), None);
    }
}
