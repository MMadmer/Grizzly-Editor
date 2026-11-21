//! A minidump made anonymous before it leaves (docs/REPORTS.md, "Privacy"), the way the game makes its
//! own crash reports' dumps (change both or neither): thread stacks and memory ranges erased, a module's
//! name and its debug record's path cut to the file name. A stream this list does not know is
//! erased whole and marked unused - a handle's name or a token says more of a machine than a crash
//! needs. What stays is the threads' registers, the modules with their versions and times, the
//! exception and the system's description: enough to find the instruction that failed.

const SIGNATURE: u32 = 0x504D_444D; // "MDMP"
/// The header's size: nothing a stream points at lies inside it.
const HEADER: usize = 32;

const THREAD_LIST: u32 = 3;
const MODULE_LIST: u32 = 4;
const MEMORY_LIST: u32 = 5;
const EXCEPTION: u32 = 6;
const SYSTEM_INFO: u32 = 7;
const THREAD_EX_LIST: u32 = 8;
const MEMORY64_LIST: u32 = 9;
const HANDLE_DATA: u32 = 12;
const FUNCTION_TABLE: u32 = 13;
const UNLOADED_MODULE_LIST: u32 = 14;
const MISC_INFO: u32 = 15;
const MEMORY_INFO_LIST: u32 = 16;
const THREAD_INFO_LIST: u32 = 17;
const SYSTEM_MEMORY_INFO: u32 = 21;
const PROCESS_VM_COUNTERS: u32 = 22;
const THREAD_NAMES: u32 = 24;

/// The streams a sanitized dump keeps: every other one is erased.
const KEPT: [u32; 15] = [
    THREAD_LIST,
    MODULE_LIST,
    MEMORY_LIST,
    EXCEPTION,
    SYSTEM_INFO,
    THREAD_EX_LIST,
    MEMORY64_LIST,
    FUNCTION_TABLE,
    UNLOADED_MODULE_LIST,
    MISC_INFO,
    MEMORY_INFO_LIST,
    THREAD_INFO_LIST,
    SYSTEM_MEMORY_INFO,
    PROCESS_VM_COUNTERS,
    THREAD_NAMES,
];

struct Dump<'a>(&'a mut [u8]);

impl Dump<'_> {
    fn u32(&self, at: usize) -> Option<u32> {
        self.0.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&self, at: usize) -> Option<u64> {
        let b = self.0.get(at..at.checked_add(8)?)?;
        Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn put_u32(&mut self, at: usize, v: u32) -> Option<()> {
        self.0.get_mut(at..at.checked_add(4)?)?.copy_from_slice(&v.to_le_bytes());
        Some(())
    }

    fn put_u64(&mut self, at: usize, v: u64) -> Option<()> {
        self.0.get_mut(at..at.checked_add(8)?)?.copy_from_slice(&v.to_le_bytes());
        Some(())
    }

    fn zero(&mut self, at: usize, len: usize) -> Option<()> {
        self.0.get_mut(at..at.checked_add(len)?)?.fill(0);
        Some(())
    }

    /// A location descriptor (`DataSize`, `Rva`) at `at`: its bytes zeroed and the descriptor emptied.
    /// One that points into the header holds nothing of its own (a stack a dump left out says its size
    /// with an `Rva` of 0): only the descriptor is emptied, the header is never touched.
    fn erase_location(&mut self, at: usize) -> Option<()> {
        let size = self.u32(at)? as usize;
        let rva = self.u32(at + 4)? as usize;
        if size > 0 && rva >= HEADER {
            self.zero(rva, size)?;
        }
        self.put_u32(at, 0)?;
        self.put_u32(at + 4, 0)
    }

    /// A `MINIDUMP_STRING` (a length in bytes, then UTF-16) cut to what follows its last slash.
    fn cut_string(&mut self, rva: usize) {
        let Some(len) = self.u32(rva).map(|l| l as usize) else { return };
        if len % 2 != 0 || self.0.len() < rva + 4 + len {
            return;
        }
        let units: Vec<u16> = self.0[rva + 4..rva + 4 + len]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let start =
            units.iter().rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/')).map_or(0, |i| i + 1);
        if start == 0 {
            return;
        }
        let name: Vec<u8> = units[start..].iter().flat_map(|u| u.to_le_bytes()).collect();
        let area = &mut self.0[rva + 4..rva + 4 + len];
        area.fill(0);
        area[..name.len()].copy_from_slice(&name);
        let _ = self.put_u32(rva, name.len() as u32);
    }

    /// A zero-ended ANSI path at `from` inside `[from, end)` cut to its file name.
    fn cut_ansi(&mut self, from: usize, end: usize) {
        let Some(area) = self.0.get_mut(from..end) else { return };
        let Some(len) = area.iter().position(|&b| b == 0) else { return };
        let Some(slash) = area[..len].iter().rposition(|&b| b == b'\\' || b == b'/') else { return };
        let name = area[slash + 1..len].to_vec();
        area.fill(0);
        area[..name.len()].copy_from_slice(&name);
    }

    /// A module's CodeView (`RSDS` / `NB10`) and misc debug records: the PDB's path cut to its name.
    fn cut_debug_paths(&mut self, module: usize) {
        const RSDS: u32 = 0x5344_5352;
        const NB10: u32 = 0x3031_424E;
        if let (Some(size), Some(rva)) = (self.u32(module + 76), self.u32(module + 80)) {
            let (size, rva) = (size as usize, rva as usize);
            match self.u32(rva) {
                Some(RSDS) if size > 24 => self.cut_ansi(rva + 24, rva + size),
                Some(NB10) if size > 16 => self.cut_ansi(rva + 16, rva + size),
                _ => {}
            }
        }
        if let (Some(size), Some(rva)) = (self.u32(module + 84), self.u32(module + 88)) {
            let (size, rva) = (size as usize, rva as usize);
            // IMAGE_DEBUG_MISC: DataType, Length, Unicode, 3 reserved, then the data; a path in UTF-16
            // is erased whole
            let ansi = self.0.get(rva + 8).is_some_and(|&u| u == 0);
            if size > 12 && ansi {
                self.cut_ansi(rva + 12, rva + size);
            } else if size > 12 && rva >= HEADER {
                let _ = self.zero(rva + 12, size - 12);
            }
        }
    }

    /// MISC_INFO_3 and later carry the time zone by its names (`TimeZoneId` at 56, the
    /// `TIME_ZONE_INFORMATION` to 232): where the machine is. Zeroed.
    fn misc_info(&mut self, rva: usize, size: usize) {
        if size >= 232 {
            let _ = self.zero(rva + 56, 232 - 56);
        }
    }

    /// A handle stream names the objects it lists in strings outside its own bytes (a file's path):
    /// those are zeroed before the stream itself goes.
    fn handle_names(&mut self, rva: usize) {
        let (Some(header), Some(entry), Some(count)) = (self.u32(rva), self.u32(rva + 4), self.u32(rva + 8))
        else {
            return;
        };
        let (header, entry, count) = (header as usize, entry as usize, count as usize);
        if entry < 16 || count > self.0.len().saturating_sub(rva + header) / entry {
            return;
        }
        for i in 0..count {
            let at = rva + header + i * entry;
            for name in [at + 8, at + 12] {
                if let Some(text) = self.u32(name).map(|r| r as usize).filter(|&r| r >= HEADER)
                    && let Some(len) = self.u32(text)
                {
                    let _ = self.zero(text, 4 + len as usize);
                }
            }
        }
    }

    fn threads(&mut self, rva: usize, size: usize, entry: usize) -> Option<()> {
        let count = self.u32(rva)? as usize;
        if size < 4 || count > (size - 4) / entry {
            return None;
        }
        for i in 0..count {
            let thread = rva + 4 + i * entry;
            // the stack: StartOfMemoryRange at 24, its location at 32
            self.put_u64(thread + 24, 0)?;
            self.erase_location(thread + 32)?;
            if entry >= 64 {
                // the backing store of a THREAD_EX
                self.put_u64(thread + 48, 0)?;
                self.erase_location(thread + 56)?;
            }
        }
        Some(())
    }

    fn modules(&mut self, rva: usize, size: usize) -> Option<()> {
        const ENTRY: usize = 108;
        let count = self.u32(rva)? as usize;
        if size < 4 || count > (size - 4) / ENTRY {
            return None;
        }
        for i in 0..count {
            let module = rva + 4 + i * ENTRY;
            if let Some(name) = self.u32(module + 20) {
                self.cut_string(name as usize);
            }
            self.cut_debug_paths(module);
        }
        Some(())
    }

    fn unloaded(&mut self, rva: usize) -> Option<()> {
        let header = self.u32(rva)? as usize;
        let entry = self.u32(rva + 4)? as usize;
        let count = self.u32(rva + 8)? as usize;
        if entry < 24 || count > self.0.len().saturating_sub(rva + header) / entry {
            return None;
        }
        for i in 0..count {
            if let Some(name) = self.u32(rva + header + i * entry + 20) {
                self.cut_string(name as usize);
            }
        }
        Some(())
    }

    fn memory(&mut self, rva: usize, size: usize) -> Option<()> {
        let count = self.u32(rva)? as usize;
        if size < 4 || count > (size - 4) / 16 {
            return None;
        }
        for i in 0..count {
            let range = rva + 4 + i * 16;
            self.put_u64(range, 0)?;
            self.erase_location(range + 8)?;
        }
        self.put_u32(rva, 0)
    }

    fn memory64(&mut self, rva: usize, size: usize) -> Option<()> {
        let count = usize::try_from(self.u64(rva)?).ok()?;
        let mut data = usize::try_from(self.u64(rva + 8)?).ok()?;
        if size < 16 || count > (size - 16) / 16 {
            return None;
        }
        for i in 0..count {
            let range = rva + 16 + i * 16;
            let len = usize::try_from(self.u64(range + 8)?).ok()?;
            self.zero(data, len)?;
            data = data.checked_add(len)?;
            self.put_u64(range, 0)?;
            self.put_u64(range + 8, 0)?;
        }
        self.put_u64(rva, 0)?;
        self.put_u64(rva + 8, 0)
    }
}

/// Makes `bytes` - a minidump - anonymous in place. A file that is no minidump, or one whose
/// structure does not hold, is refused: such a dump is not sent at all.
pub fn sanitize(bytes: &mut [u8]) -> Result<(), &'static str> {
    let mut d = Dump(bytes);
    if d.u32(0) != Some(SIGNATURE) {
        return Err("the dump is no minidump");
    }
    let broken = "the dump's structure does not hold";
    let count = d.u32(8).ok_or(broken)? as usize;
    let directory = d.u32(12).ok_or(broken)? as usize;
    if count > d.0.len().saturating_sub(directory) / 12 {
        return Err(broken);
    }
    // the directory is read before anything is changed: no zeroing can hide a stream from this pass
    let mut streams = Vec::with_capacity(count);
    for i in 0..count {
        let at = directory + i * 12;
        let kind = d.u32(at).ok_or(broken)?;
        let size = d.u32(at + 4).ok_or(broken)? as usize;
        let rva = d.u32(at + 8).ok_or(broken)? as usize;
        streams.push((at, kind, size, rva));
    }
    for (at, kind, size, rva) in streams {
        if size == 0 {
            continue;
        }
        if rva < HEADER || rva.checked_add(size).is_none_or(|end| end > d.0.len()) {
            return Err(broken);
        }
        let done = match kind {
            THREAD_LIST => d.threads(rva, size, 48),
            THREAD_EX_LIST => d.threads(rva, size, 64),
            MODULE_LIST => d.modules(rva, size),
            UNLOADED_MODULE_LIST => d.unloaded(rva),
            MEMORY_LIST => d.memory(rva, size),
            MEMORY64_LIST => d.memory64(rva, size),
            MISC_INFO => {
                d.misc_info(rva, size);
                Some(())
            }
            k if KEPT.contains(&k) => Some(()),
            k => {
                if k == HANDLE_DATA {
                    d.handle_names(rva);
                }
                // UnusedStream: a reader skips it
                d.zero(rva, size);
                d.put_u32(at, 0)
            }
        };
        done.ok_or(broken)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// A dump of one thread with a stack, one module with a name and a PDB path, and a handle stream.
    fn sample() -> (Vec<u8>, usize, usize, usize) {
        let mut d = vec![0u8; 32];
        d[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        // the stack's bytes
        let stack = d.len();
        d.extend_from_slice(b"secret stack bytes of C:\\Users\\Ivan");
        // the module's name
        let name = d.len();
        // program-files: in memory
        let path = utf16("C:\\Users\\Ivan\\AppData\\Local\\Programs\\Grizzly Editor\\Grizzly.exe");
        d.extend_from_slice(&(path.len() as u32).to_le_bytes());
        d.extend_from_slice(&path);
        d.extend_from_slice(&[0, 0]);
        // its CodeView record
        let cv = d.len();
        d.extend_from_slice(&0x5344_5352_u32.to_le_bytes());
        d.extend_from_slice(&[7; 20]);
        d.extend_from_slice(b"D:\\Work\\Ivan\\target\\grizzly.pdb\0");
        let cv_len = d.len() - cv;
        // the thread list
        let threads = d.len();
        d.extend_from_slice(&1_u32.to_le_bytes());
        let mut t = vec![0u8; 48];
        t[24..32].copy_from_slice(&0x1000_u64.to_le_bytes());
        t[32..36].copy_from_slice(&35_u32.to_le_bytes());
        t[36..40].copy_from_slice(&(stack as u32).to_le_bytes());
        d.extend_from_slice(&t);
        // the module list
        let modules = d.len();
        d.extend_from_slice(&1_u32.to_le_bytes());
        let mut m = vec![0u8; 108];
        m[20..24].copy_from_slice(&(name as u32).to_le_bytes());
        m[76..80].copy_from_slice(&(cv_len as u32).to_le_bytes());
        m[80..84].copy_from_slice(&(cv as u32).to_le_bytes());
        d.extend_from_slice(&m);
        // a handle stream, which no anonymous dump keeps
        let handles = d.len();
        d.extend_from_slice(b"\\Device\\HarddiskVolume3\\Users\\Ivan\\x");
        // the directory
        let directory = d.len();
        for (kind, rva, size) in [(THREAD_LIST, threads, 52), (MODULE_LIST, modules, 112), (12, handles, 36)]
        {
            d.extend_from_slice(&kind.to_le_bytes());
            d.extend_from_slice(&(size as u32).to_le_bytes());
            d.extend_from_slice(&(rva as u32).to_le_bytes());
        }
        d[8..12].copy_from_slice(&3_u32.to_le_bytes());
        d[12..16].copy_from_slice(&(directory as u32).to_le_bytes());
        (d, name, cv, directory)
    }

    fn holds(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn stacks_paths_and_unknown_streams_go() {
        let (mut d, name, cv, directory) = sample();
        assert!(sanitize(&mut d).is_ok());
        assert!(!holds(&d, b"Ivan"), "an ANSI trace of the user is left");
        assert!(!holds(&d, &utf16("Ivan")), "a UTF-16 trace of the user is left");
        assert!(!holds(&d, b"secret stack"));
        // the module keeps its file name, the PDB its own
        let len = u32::from_le_bytes([d[name], d[name + 1], d[name + 2], d[name + 3]]) as usize;
        // program-files: in memory
        assert_eq!(&d[name + 4..name + 4 + len], utf16("Grizzly.exe").as_slice());
        assert!(d[cv + 24..].starts_with(b"grizzly.pdb\0"));
        // the handle stream is unused now
        assert_eq!(&d[directory + 24..directory + 28], &0_u32.to_le_bytes());
    }

    #[test]
    fn a_stack_left_out_wipes_nothing_and_the_time_zone_goes() {
        let (mut d, name, _, _) = sample();
        // the thread's stack: its size kept, its Rva 0, as a dump that removed the memory writes it
        let directory = u32::from_le_bytes([d[12], d[13], d[14], d[15]]) as usize;
        let at = directory + 8;
        let threads = u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as usize;
        d[threads + 4 + 36..threads + 4 + 40].copy_from_slice(&0_u32.to_le_bytes());
        // a misc info stream with a time zone name in it
        let misc = d.len();
        let mut m = vec![0u8; 232];
        m[0..4].copy_from_slice(&232_u32.to_le_bytes());
        m[64..64 + 22].copy_from_slice(&utf16("Moscow Time"));
        d.extend_from_slice(&m);
        let mut dir = d[directory..directory + 36].to_vec();
        dir.extend_from_slice(&MISC_INFO.to_le_bytes());
        dir.extend_from_slice(&232_u32.to_le_bytes());
        dir.extend_from_slice(&(misc as u32).to_le_bytes());
        let new_dir = d.len();
        d.extend_from_slice(&dir);
        d[8..12].copy_from_slice(&4_u32.to_le_bytes());
        d[12..16].copy_from_slice(&(new_dir as u32).to_le_bytes());
        assert!(sanitize(&mut d).is_ok());
        assert_eq!(&d[0..4], &SIGNATURE.to_le_bytes(), "the header is kept");
        let len = u32::from_le_bytes([d[name], d[name + 1], d[name + 2], d[name + 3]]) as usize;
        // program-files: in memory
        assert_eq!(
            &d[name + 4..name + 4 + len],
            utf16("Grizzly.exe").as_slice(),
            "the module list was still cut"
        );
        assert!(!holds(&d, &utf16("Moscow")), "the time zone is gone");
    }

    /// The names of a dump's modules, as its module list holds them.
    fn module_names(d: &[u8]) -> Vec<String> {
        let at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as usize;
        let (count, directory) = (at(8), at(12));
        let mut names = Vec::new();
        for i in 0..count {
            let entry = directory + i * 12;
            if at(entry) != MODULE_LIST as usize {
                continue;
            }
            let rva = at(entry + 8);
            for m in 0..at(rva) {
                let name = at(rva + 4 + m * 108 + 20);
                let len = at(name);
                let units: Vec<u16> = d[name + 4..name + 4 + len]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                names.push(String::from_utf16_lossy(&units));
            }
        }
        names
    }

    /// A real dump of this process, as the editor writes one after a fault: what this pass does to it.
    #[cfg(windows)]
    #[test]
    fn a_real_compact_dump_keeps_its_modules_by_name() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("probe.dmp");
        gz_win::write_minidump(&path, true).unwrap_or_else(|e| panic!("the dump was not written: {e}"));
        let mut d = std::fs::read(&path).unwrap_or_default();
        assert!(sanitize(&mut d).is_ok(), "a dump the editor writes is one this pass reads");
        let names = module_names(&d);
        assert!(names.len() > 3, "{names:?}");
        assert!(names.iter().all(|n| !n.contains('\\') && !n.contains('/')), "{names:?}");
        let exe = std::env::current_exe().unwrap_or_default();
        let folder = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        assert!(!holds(&d, &utf16(&folder)), "the program's folder is in the dump");
        assert_eq!(&d[0..4], &SIGNATURE.to_le_bytes());
    }

    #[test]
    fn what_is_no_dump_is_refused() {
        assert!(sanitize(&mut b"not a dump at all, sorry".to_vec()).is_err());
        let (mut d, ..) = sample();
        // a directory that points past the end
        let at = d.len() - 36;
        d[at + 8..at + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(sanitize(&mut d).is_err());
    }
}
