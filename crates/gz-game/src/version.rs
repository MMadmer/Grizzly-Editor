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
