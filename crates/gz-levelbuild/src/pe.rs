//! The libraries a Windows program names in its import tables (the ordinary and the delay-load
//! ones): what Play reads to know which libraries of the game's folder its engine needs. Only the
//! headers and the import tables are read, never the whole file.

use std::io::{Read, Seek, SeekFrom};

const IMPORT_DIR: usize = 1;
const DELAY_IMPORT_DIR: usize = 13;
const MAX_DESCRIPTORS: usize = 4096;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn read_at(r: &mut (impl Read + Seek), at: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(at)).ok()?;
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return None,
        }
    }
    buf.truncate(got);
    Some(buf)
}

/// The library names of `r`'s import and delay-import tables, in table order; empty for what is
/// not a program or cannot be read.
pub fn imports(r: &mut (impl Read + Seek)) -> Vec<String> {
    imports_of(r).unwrap_or_default()
}

fn imports_of(r: &mut (impl Read + Seek)) -> Option<Vec<String>> {
    let dos = read_at(r, 0, 0x40)?;
    if dos.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u64::from(u32_at(&dos, 0x3C)?);
    let head = read_at(r, pe, 24)?;
    if head.get(..4)? != b"PE\0\0" {
        return None;
    }
    let sections = usize::from(u16_at(&head, 6)?);
    let opt_size = usize::from(u16_at(&head, 20)?);
    let opt = read_at(r, pe + 24, opt_size)?;
    let (count_at, dirs_at) = match u16_at(&opt, 0)? {
        0x20B => (108, 112),
        0x10B => (92, 96),
        _ => return None,
    };
    let dir_count = u32_at(&opt, count_at)? as usize;
    let dir = |i: usize| (i < dir_count).then(|| u32_at(&opt, dirs_at + 8 * i)).flatten().unwrap_or(0);
    let table = read_at(r, pe + 24 + opt_size as u64, sections * 40)?;
    let spans: Vec<(u32, u32, u32)> = (0..sections)
        .filter_map(|i| {
            let s = i * 40;
            let (vsize, va, raw, ptr) = (
                u32_at(&table, s + 8)?,
                u32_at(&table, s + 12)?,
                u32_at(&table, s + 16)?,
                u32_at(&table, s + 20)?,
            );
            Some((va, vsize.max(raw), ptr))
        })
        .collect();
    let offset = |rva: u32| {
        spans
            .iter()
            .find(|(va, size, _)| rva >= *va && rva - va < *size)
            .map(|(va, _, ptr)| u64::from(*ptr) + u64::from(rva - va))
    };
    let mut out = Vec::new();
    for (index, step, name_at) in [(IMPORT_DIR, 20usize, 12usize), (DELAY_IMPORT_DIR, 32, 4)] {
        let rva = dir(index);
        let Some(mut at) = (rva != 0).then(|| offset(rva)).flatten() else { continue };
        for _ in 0..MAX_DESCRIPTORS {
            let Some(desc) = read_at(r, at, step).filter(|d| d.len() == step) else { break };
            if desc.iter().all(|b| *b == 0) {
                break;
            }
            if let Some(name) = u32_at(&desc, name_at).and_then(offset).and_then(|o| read_at(r, o, 260)) {
                let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
                if let Ok(n) = std::str::from_utf8(&name[..end])
                    && !n.is_empty()
                {
                    out.push(n.to_owned());
                }
            }
            at += step as u64;
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn put32(b: &mut [u8], at: usize, v: u32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A PE32+ image of one section in memory, with an import and a delay-import table.
    fn image() -> Vec<u8> {
        let mut b = vec![0u8; 0x600];
        // program-files: in memory
        b[0..2].copy_from_slice(b"MZ");
        put32(&mut b, 0x3C, 0x80);
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        put32(&mut b, opt + 108, 16);
        // the import table at RVA 0x1000, the delay imports at 0x1100
        put32(&mut b, opt + 112 + 8, 0x1000);
        put32(&mut b, opt + 112 + 8 * 13, 0x1100);
        let sec = opt + 240;
        put32(&mut b, sec + 8, 0x400);
        put32(&mut b, sec + 12, 0x1000);
        put32(&mut b, sec + 16, 0x400);
        put32(&mut b, sec + 20, 0x200);
        // two import descriptors and the empty one, then a delay descriptor
        put32(&mut b, 0x200 + 12, 0x1200);
        put32(&mut b, 0x200 + 20 + 12, 0x1210);
        put32(&mut b, 0x300 + 4, 0x1220);
        b[0x400..0x40C].copy_from_slice(b"xrCore.lib\0\0");
        b[0x410..0x41D].copy_from_slice(b"KERNEL32.sys\0");
        b[0x420..0x432].copy_from_slice(b"xrRender_R6.lib\0\0\0");
        b
    }

    #[test]
    fn the_import_and_delay_import_tables_are_read() {
        let names = imports(&mut Cursor::new(image()));
        assert_eq!(names, ["xrCore.lib", "KERNEL32.sys", "xrRender_R6.lib"]);
    }

    #[test]
    fn what_is_not_a_program_has_no_imports() {
        assert!(imports(&mut Cursor::new(b"engine".to_vec())).is_empty());
        let mut cut = image();
        cut.truncate(0x90);
        assert!(imports(&mut Cursor::new(cut)).is_empty());
    }
}
