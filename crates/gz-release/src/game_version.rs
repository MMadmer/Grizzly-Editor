//! The linked game's version (`LinkedGameVersion`): `MAJOR.MINOR.PATCH` of the file version
//! `xrEngine.exe` carries in its version resource - what a release asks for as `requires_game`
//! when nothing else is given. Read from the PE file itself: the headers, then the resource
//! section only.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const RT_VERSION: u32 = 16;
const FIXED_SIGNATURE: u32 = 0xFEEF_04BD;
/// The largest resource section read.
const RSRC_LIMIT: u64 = 64 << 20;

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

/// The resource section: its bytes, the RVA they start at, and where the resource tree's root
/// lies in them (the offsets of the tree count from there).
struct Rsrc {
    bytes: Vec<u8>,
    rva: u32,
    root: usize,
}

impl Rsrc {
    fn at_rva(&self, rva: u32, len: usize) -> Option<&[u8]> {
        let start = rva.checked_sub(self.rva)? as usize;
        self.bytes.get(start..start.checked_add(len)?)
    }

    /// The entry of a resource directory at `dir` (an offset into the section) with `id`, or
    /// its first entry.
    fn entry(&self, dir: usize, id: Option<u32>) -> Option<u32> {
        let dir = self.root + dir;
        let named = usize::from(le16(&self.bytes, dir + 12)?);
        let ids = usize::from(le16(&self.bytes, dir + 14)?);
        (0..named + ids).find_map(|i| {
            let at = dir + 16 + i * 8;
            let (name, target) = (le32(&self.bytes, at)?, le32(&self.bytes, at + 4)?);
            match id {
                Some(id) if name & 0x8000_0000 != 0 || name != id => None,
                _ => Some(target),
            }
        })
    }
}

fn read_rsrc(file: &mut File) -> Option<Rsrc> {
    let mut head = vec![0u8; 4096];
    let n = file.read(&mut head).ok()?;
    head.truncate(n);
    if !head.starts_with(b"MZ") {
        return None;
    }
    let pe = le32(&head, 0x3C)? as usize;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = usize::from(le16(&head, coff + 2)?);
    let opt_size = usize::from(le16(&head, coff + 16)?);
    let opt = coff + 20;
    let dirs = match le16(&head, opt)? {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return None,
    };
    let rsrc_rva = le32(&head, dirs + 2 * 8)?;
    if rsrc_rva == 0 {
        return None;
    }
    let table = opt + opt_size;
    for s in 0..sections {
        let at = table + s * 40;
        let (vsize, va, raw_size, raw_at) =
            (le32(&head, at + 8)?, le32(&head, at + 12)?, le32(&head, at + 16)?, le32(&head, at + 20)?);
        if rsrc_rva < va || rsrc_rva >= va.saturating_add(vsize.max(raw_size)) {
            continue;
        }
        let len = u64::from(raw_size);
        if len > RSRC_LIMIT {
            return None;
        }
        file.seek(SeekFrom::Start(u64::from(raw_at))).ok()?;
        let mut bytes = vec![0u8; len as usize];
        file.read_exact(&mut bytes).ok()?;
        return Some(Rsrc { bytes, rva: va, root: (rsrc_rva - va) as usize });
    }
    None
}

/// `MAJOR.MINOR.PATCH` of an executable's file version; `None` when it carries none.
#[must_use]
pub fn exe_version(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let rsrc = read_rsrc(&mut file)?;
    // the resource tree: type, name, language
    let dir_of = |target: u32| (target & 0x8000_0000 != 0).then_some((target & 0x7FFF_FFFF) as usize);
    let types = dir_of(rsrc.entry(0, Some(RT_VERSION))?)?;
    let names = dir_of(rsrc.entry(types, None)?)?;
    let leaf = rsrc.entry(names, None)?;
    if leaf & 0x8000_0000 != 0 {
        return None;
    }
    let data_entry = rsrc.root + leaf as usize;
    let (data_rva, data_size) = (le32(&rsrc.bytes, data_entry)?, le32(&rsrc.bytes, data_entry + 4)?);
    let data = rsrc.at_rva(data_rva, data_size as usize)?;
    // VS_FIXEDFILEINFO follows the VS_VERSION_INFO key, on a four-byte boundary
    let fixed =
        (0..data.len().saturating_sub(16)).step_by(4).find(|&i| le32(data, i) == Some(FIXED_SIGNATURE))?;
    let (ms, ls) = (le32(data, fixed + 8)?, le32(data, fixed + 12)?);
    Some(format!("{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16))
}

/// The linked game's version: `<game>\xrEngine.exe`. Empty without a game or a version.
#[must_use]
pub fn linked_game_version(game_root: &str) -> String {
    if game_root.is_empty() {
        return String::new();
    }
    exe_version(&Path::new(game_root).join("xrEngine.exe")).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A minimal PE32+ with one section holding a version resource.
    fn fake_exe(ms: u32, ls: u32) -> Vec<u8> {
        let mut f = vec![0u8; 0x400];
        f[0] = b'M';
        f[1] = b'Z';
        f[0x3C] = 0x80;
        f[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        f[coff + 2] = 1; // one section
        f[coff + 16] = 240; // optional header size
        let opt = coff + 20;
        f[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        let dirs = opt + 112;
        f[dirs + 16..dirs + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        let sec = opt + 240;
        f[sec..sec + 5].copy_from_slice(b".rsrc");
        f[sec + 8..sec + 12].copy_from_slice(&0x200u32.to_le_bytes()); // virtual size
        f[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // RVA
        f[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes()); // raw size
        f[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes()); // raw offset
        let r = 0x200;
        let put32 = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
        // root: one id entry, RT_VERSION -> directory at 0x18
        f[r + 14] = 1;
        put32(&mut f, r + 16, RT_VERSION);
        put32(&mut f, r + 20, 0x8000_0018);
        // names: one entry -> directory at 0x30
        f[r + 0x18 + 14] = 1;
        put32(&mut f, r + 0x18 + 16, 1);
        put32(&mut f, r + 0x18 + 20, 0x8000_0030);
        // languages: one entry -> data entry at 0x48
        f[r + 0x30 + 14] = 1;
        put32(&mut f, r + 0x30 + 16, 0x409);
        put32(&mut f, r + 0x30 + 20, 0x48);
        // the data entry: RVA 0x1060, 0x60 bytes
        put32(&mut f, r + 0x48, 0x1060);
        put32(&mut f, r + 0x4C, 0x60);
        // VS_VERSION_INFO: the fixed info at a four-byte boundary past the key
        put32(&mut f, r + 0x60 + 40, FIXED_SIGNATURE);
        put32(&mut f, r + 0x60 + 48, ms);
        put32(&mut f, r + 0x60 + 52, ls);
        f
    }

    #[test]
    fn reads_the_file_version() {
        let dir = std::env::temp_dir().join(format!("gz-release-ver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("xrEngine.exe"), fake_exe((1 << 16) | 4, 2 << 16 | 7)).unwrap();
        assert_eq!(linked_game_version(&dir.to_string_lossy()), "1.4.2");
        std::fs::write(dir.join("xrEngine.exe"), b"not a program").unwrap();
        assert_eq!(linked_game_version(&dir.to_string_lossy()), "");
        assert_eq!(linked_game_version(""), "");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
