//! The linked game's version: the file version of its `xrEngine.exe`, `major.minor.build`
//! (`EditorMod::LinkedGameVersion`, which asked `GetFileVersionInfo`).
//!
//! The version resource holds a `VS_FIXEDFILEINFO`: signature `0xFEEF04BD`, structure version
//! `0x00010000`, then `dwFileVersionMS` and `dwFileVersionLS`. The first such block of the file
//! is the one `VerQueryValue(..., "\\")` answers.

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

#[cfg(test)]
mod tests {
    use super::*;

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
