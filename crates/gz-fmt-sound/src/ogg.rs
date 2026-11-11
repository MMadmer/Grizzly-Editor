//! Ogg pages: enough to find a Vorbis stream's header packets and to lay them out again.

use crate::{Result, SoundError};

const PAGE_HEADER: usize = 27;
/// A page that continues the packet the page before left open.
const CONTINUED: u8 = 0x01;

/// One page, borrowed from the file.
#[derive(Clone, Debug)]
pub(crate) struct Page<'a> {
    /// Where the page starts in the file, and how many bytes it takes.
    pub start: usize,
    pub len: usize,
    pub granule: u64,
    pub serial: u32,
    pub lacing: &'a [u8],
    pub body: &'a [u8],
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
            bit += 1;
        }
        table[i] = r;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = crc_table();

/// The checksum of a page whose checksum field is zero.
pub(crate) fn crc(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |c, &b| (c << 8) ^ CRC_TABLE[usize::from((c >> 24) as u8 ^ b)])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    let mut raw = [0u8; 4];
    if let Some(src) = bytes.get(at..at + 4) {
        raw.copy_from_slice(src);
    }
    u32::from_le_bytes(raw)
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    if let Some(src) = bytes.get(at..at + 8) {
        raw.copy_from_slice(src);
    }
    u64::from_le_bytes(raw)
}

/// Every whole page of the file, in order. Bytes that are no page - junk between pages, a tail
/// cut short - are skipped; a file without a single page is not an Ogg file.
pub(crate) fn parse_pages(data: &[u8]) -> Result<Vec<Page<'_>>> {
    let mut pages = Vec::new();
    let mut pos = 0usize;
    while let Some(head) = data.get(pos..pos + PAGE_HEADER) {
        if &head[..4] != b"OggS" || head[4] != 0 {
            // resynchronise on the next capture pattern
            match data[pos + 1..].windows(4).position(|w| w == b"OggS") {
                Some(skip) => {
                    pos += 1 + skip;
                    continue;
                }
                None => break,
            }
        }
        let segments = usize::from(head[26]);
        let Some(lacing) = data.get(pos + PAGE_HEADER..pos + PAGE_HEADER + segments) else { break };
        let body_len: usize = lacing.iter().map(|&v| usize::from(v)).sum();
        let body_at = pos + PAGE_HEADER + segments;
        let Some(body) = data.get(body_at..body_at + body_len) else { break };
        pages.push(Page {
            start: pos,
            len: PAGE_HEADER + segments + body_len,
            granule: le64(head, 6),
            serial: le32(head, 14),
            lacing,
            body,
        });
        pos = body_at + body_len;
    }
    if pages.is_empty() {
        return Err(SoundError::NotOgg("it has no Ogg page"));
    }
    Ok(pages)
}

/// The three Vorbis header packets of the file's first stream, and where they lie.
#[derive(Clone, Debug)]
pub(crate) struct Headers {
    pub serial: u32,
    pub ident: Vec<u8>,
    pub comment: Vec<u8>,
    pub setup: Vec<u8>,
    /// Indices into the page list of the stream's pages that carry the comment and setup
    /// packets, first and last.
    pub first_page: usize,
    pub last_page: usize,
    /// The identification packet is alone on the first page, and the setup packet ends its
    /// page: what the Vorbis mapping requires, and what makes the pages replaceable.
    pub clean: bool,
}

/// Collects the header packets of the first stream.
pub(crate) fn read_headers(pages: &[Page<'_>]) -> Result<Headers> {
    let serial = pages.first().map(|p| p.serial).ok_or(SoundError::NotOgg("it has no Ogg page"))?;
    let mut packets: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut current = 0usize;
    let mut first_page = None;
    let mut ident_alone = false;

    for (index, page) in pages.iter().enumerate().filter(|(_, p)| p.serial == serial) {
        if current >= 1 && first_page.is_none() {
            first_page = Some(index);
        }
        let mut at = 0usize;
        for (seg, &lace) in page.lacing.iter().enumerate() {
            let len = usize::from(lace);
            // the lacing table was summed when the page was parsed, so this stays inside the body
            let Some(part) = page.body.get(at..at + len) else {
                return Err(SoundError::Damaged("a page is shorter than its lacing"));
            };
            at += len;
            if let Some(packet) = packets.get_mut(current) {
                packet.extend_from_slice(part);
            }
            if lace < 255 {
                let last_segment = seg + 1 == page.lacing.len();
                match current {
                    0 => ident_alone = last_segment,
                    2 => {
                        let [ident, comment, setup] = packets;
                        return Ok(Headers {
                            serial,
                            ident,
                            comment,
                            setup,
                            first_page: first_page.unwrap_or(index),
                            last_page: index,
                            clean: ident_alone && last_segment,
                        });
                    }
                    _ => {}
                }
                current += 1;
            }
        }
    }
    Err(SoundError::Damaged("the Vorbis headers are incomplete"))
}

/// Lays packets out on pages the way libogg flushes them: 255 segments a page at most, a packet
/// of a multiple of 255 bytes closed by an empty segment. Header pages carry granule 0 where a
/// packet ends and -1 where none does. Returns the number of pages written.
pub(crate) fn write_pages(packets: &[&[u8]], serial: u32, first_sequence: u32, out: &mut Vec<u8>) -> u32 {
    // (lacing value, the segment closes a packet)
    let mut segments: Vec<(u8, bool)> = Vec::new();
    let mut data: Vec<u8> = Vec::with_capacity(packets.iter().map(|p| p.len()).sum());
    for packet in packets {
        segments.extend(std::iter::repeat_n((255u8, false), packet.len() / 255));
        segments.push(((packet.len() % 255) as u8, true));
        data.extend_from_slice(packet);
    }

    let mut written = 0u32;
    let mut body_at = 0usize;
    let mut open_packet = false;
    for group in segments.chunks(255) {
        let body_len: usize = group.iter().map(|&(v, _)| usize::from(v)).sum();
        let ends_packet = group.iter().any(|&(_, end)| end);
        let page_at = out.len();

        out.extend_from_slice(b"OggS");
        out.push(0);
        out.push(if open_packet { CONTINUED } else { 0 });
        out.extend_from_slice(&(if ends_packet { 0u64 } else { u64::MAX }).to_le_bytes());
        out.extend_from_slice(&serial.to_le_bytes());
        out.extend_from_slice(&first_sequence.wrapping_add(written).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.push(group.len() as u8);
        out.extend(group.iter().map(|&(v, _)| v));
        out.extend_from_slice(data.get(body_at..body_at + body_len).unwrap_or(&[]));
        body_at += body_len;

        let sum = crc(&out[page_at..]);
        out[page_at + 22..page_at + 26].copy_from_slice(&sum.to_le_bytes());

        open_packet = group.last().is_some_and(|&(_, end)| !end);
        written += 1;
    }
    written
}

/// A page's sequence number, read from its bytes.
pub(crate) fn sequence_of(page_bytes: &[u8]) -> u32 {
    le32(page_bytes, 18)
}

/// Copies a page with another sequence number and the checksum that goes with it.
pub(crate) fn write_renumbered(page_bytes: &[u8], sequence: u32, out: &mut Vec<u8>) {
    let at = out.len();
    out.extend_from_slice(page_bytes);
    if page_bytes.len() >= PAGE_HEADER {
        out[at + 18..at + 22].copy_from_slice(&sequence.to_le_bytes());
        out[at + 22..at + 26].copy_from_slice(&[0u8; 4]);
        let sum = crc(&out[at..]);
        out[at + 22..at + 26].copy_from_slice(&sum.to_le_bytes());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_oggs() {
        // CRC-32 with polynomial 0x04c11db7, no reflection, zero start: the value for "123456789"
        assert_eq!(crc(b"123456789"), 0x89a1_897f);
        assert_eq!(crc(&[]), 0);
    }

    #[test]
    fn packets_come_back_from_the_pages_they_were_laid_on() {
        let ident = vec![1u8; 30];
        let comment = vec![2u8; 255 * 3];
        let setup = vec![3u8; 255 * 300 + 17];
        let mut file = Vec::new();
        assert_eq!(write_pages(&[&ident], 77, 0, &mut file), 1);
        // 3 + 1 segments for the comment, 301 for the setup: two pages
        assert_eq!(write_pages(&[&comment, &setup], 77, 1, &mut file), 2);

        let pages = parse_pages(&file).unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[1].granule, 0);
        assert_eq!(pages[2].granule, 0);
        assert_eq!(sequence_of(&file[pages[2].start..]), 2);
        // the third page continues the setup packet
        assert_eq!(file[pages[2].start + 5], CONTINUED);
        for page in &pages {
            let mut copy = file[page.start..page.start + page.len].to_vec();
            let stored = le32(&copy, 22);
            copy[22..26].copy_from_slice(&[0; 4]);
            assert_eq!(crc(&copy), stored);
        }

        let headers = read_headers(&pages).unwrap();
        assert_eq!(headers.serial, 77);
        assert_eq!((headers.ident, headers.comment, headers.setup), (ident, comment, setup));
        assert_eq!((headers.first_page, headers.last_page, headers.clean), (1, 2, true));
    }

    #[test]
    fn junk_and_short_tails_are_skipped() {
        let mut file = b"junk".to_vec();
        write_pages(&[&[9u8; 10]], 1, 0, &mut file);
        file.extend_from_slice(b"OggS\0\0");
        let pages = parse_pages(&file).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].start, 4);
        assert!(matches!(parse_pages(b"not an ogg file at all, just text"), Err(SoundError::NotOgg(_))));
        assert!(matches!(read_headers(&pages), Err(SoundError::Damaged(_))));
    }

    #[test]
    fn a_page_can_be_renumbered() {
        let mut file = Vec::new();
        write_pages(&[&[5u8; 40]], 3, 7, &mut file);
        let mut out = Vec::new();
        write_renumbered(&file, 9, &mut out);
        assert_eq!(sequence_of(&out), 9);
        let stored = le32(&out, 22);
        out[22..26].copy_from_slice(&[0; 4]);
        assert_eq!(crc(&out), stored);
    }
}
