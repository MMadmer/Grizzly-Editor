//! A small ZIP of a report (docs/REPORTS.md, "The package"): deflated entries, UTF-8 names, one fixed
//! time, in memory - a report is a few megabytes at most.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::DeflateEncoder;

struct Entry {
    name: String,
    crc: u32,
    size: u32,
    packed: u32,
    offset: u32,
}

/// A ZIP being written into memory.
#[derive(Default)]
pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

// the oldest time a ZIP entry can carry: a report says its own time in report.json
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

impl Zip {
    /// Adds `bytes` as `name` (forward slashes). An entry past 4 GiB has no business in a report.
    pub fn add(&mut self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes)?;
        let packed = encoder.finish()?;
        let too_big = |n: usize| {
            u32::try_from(n)
                .map_err(|_| std::io::Error::other("a report entry is larger than a ZIP of its kind holds"))
        };
        let entry = Entry {
            name: name.to_owned(),
            crc: crc32fast::hash(bytes),
            size: too_big(bytes.len())?,
            packed: too_big(packed.len())?,
            offset: too_big(self.data.len())?,
        };
        let d = &mut self.data;
        d.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        d.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        d.extend_from_slice(&(1_u16 << 11).to_le_bytes()); // UTF-8 names
        d.extend_from_slice(&8_u16.to_le_bytes()); // deflate
        d.extend_from_slice(&DOS_TIME.to_le_bytes());
        d.extend_from_slice(&DOS_DATE.to_le_bytes());
        d.extend_from_slice(&entry.crc.to_le_bytes());
        d.extend_from_slice(&entry.packed.to_le_bytes());
        d.extend_from_slice(&entry.size.to_le_bytes());
        d.extend_from_slice(&too_big(name.len()).map(|n| n as u16)?.to_le_bytes());
        d.extend_from_slice(&0_u16.to_le_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(&packed);
        self.entries.push(entry);
        Ok(())
    }

    /// The bytes written so far plus the central directory a finished archive would get: the size it
    /// would have.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len() + self.entries.iter().map(|e| 46 + e.name.len()).sum::<usize>() + 22
    }

    /// The finished archive.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let directory = self.data.len();
        for e in &self.entries {
            let d = &mut self.data;
            d.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            d.extend_from_slice(&20_u16.to_le_bytes()); // made by
            d.extend_from_slice(&20_u16.to_le_bytes()); // needed
            d.extend_from_slice(&(1_u16 << 11).to_le_bytes());
            d.extend_from_slice(&8_u16.to_le_bytes());
            d.extend_from_slice(&DOS_TIME.to_le_bytes());
            d.extend_from_slice(&DOS_DATE.to_le_bytes());
            d.extend_from_slice(&e.crc.to_le_bytes());
            d.extend_from_slice(&e.packed.to_le_bytes());
            d.extend_from_slice(&e.size.to_le_bytes());
            d.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            d.extend_from_slice(&[0; 12]); // extra, comment, disk, internal and external attributes
            d.extend_from_slice(&e.offset.to_le_bytes());
            d.extend_from_slice(e.name.as_bytes());
        }
        let size = self.data.len() - directory;
        let count = self.entries.len() as u16;
        let d = &mut self.data;
        d.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        d.extend_from_slice(&[0; 4]);
        d.extend_from_slice(&count.to_le_bytes());
        d.extend_from_slice(&count.to_le_bytes());
        d.extend_from_slice(&(size as u32).to_le_bytes());
        d.extend_from_slice(&(directory as u32).to_le_bytes());
        d.extend_from_slice(&0_u16.to_le_bytes());
        self.data
    }
}

/// The entries of a ZIP this module wrote: (name, uncompressed bytes) - for the tests and the dry run.
#[cfg(test)]
pub fn read(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read as _;
    let mut out = Vec::new();
    let mut at = 0;
    while bytes.len() >= at + 30 && bytes[at..at + 4] == 0x0403_4b50_u32.to_le_bytes() {
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let u32_at =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let packed = u32_at(at + 18);
        let name_len = u16_at(at + 26);
        let extra = u16_at(at + 28);
        let name = String::from_utf8_lossy(&bytes[at + 30..at + 30 + name_len]).into_owned();
        let start = at + 30 + name_len + extra;
        let mut data = Vec::new();
        flate2::read::DeflateDecoder::new(&bytes[start..start + packed]).read_to_end(&mut data).unwrap_or(0);
        assert_eq!(crc32fast::hash(&data) as usize, u32_at(at + 14), "{name}");
        out.push((name, data));
        at = start + packed;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_added_reads_back() {
        let mut z = Zip::default();
        z.add("report.json", b"{\"a\": 1}").unwrap_or_default();
        z.add("logs/editor.log", "строка лога\n".repeat(100).as_bytes()).unwrap_or_default();
        let predicted = z.len();
        let bytes = z.finish();
        assert_eq!(bytes.len(), predicted);
        let entries = read(&bytes);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ("report.json".to_owned(), b"{\"a\": 1}".to_vec()));
        assert!(String::from_utf8_lossy(&entries[1].1).starts_with("строка"));
        assert_eq!(&bytes[bytes.len() - 22..bytes.len() - 18], &0x0605_4b50_u32.to_le_bytes());
    }
}
