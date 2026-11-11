//! The game's parameters inside an `.ogg`: the first user comment of the Vorbis comment header.

use gz_chunk::{Reader, Writer};

use crate::ogg::{self, Headers};
use crate::thm::SoundThm;
use crate::{Result, SoundError};

/// The version of the comment block the editor writes.
pub const OGG_COMMENT_VERSION: u32 = 0x0003;

/// What the engine plays a sound with (`CSoundRender_Source::LoadWave`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct OggParams {
    pub min_dist: f32,
    pub max_dist: f32,
    pub base_volume: f32,
    /// The engine's sound type (`ESoundTypes`).
    pub game_type: u32,
    /// How far NPCs hear the sound.
    pub max_ai_dist: f32,
}

impl From<&SoundThm> for OggParams {
    /// What the editor puts into the game sound made from a description (`MakeGameSound`).
    fn from(thm: &SoundThm) -> Self {
        Self {
            min_dist: thm.min_dist,
            max_dist: thm.max_dist,
            base_volume: thm.base_volume,
            game_type: thm.game_type,
            max_ai_dist: thm.max_ai_dist,
        }
    }
}

impl OggParams {
    /// Reads the comment block, any version the game reads.
    pub fn from_comment(comment: &[u8]) -> Result<Self> {
        let mut r = Reader::new(comment);
        let version = r.r_u32()?;
        match version {
            1 => {
                let min_dist = r.r_f32()?;
                let max_dist = r.r_f32()?;
                let game_type = r.r_u32()?;
                Ok(Self { min_dist, max_dist, base_volume: 1.0, game_type, max_ai_dist: max_dist })
            }
            2 => {
                let min_dist = r.r_f32()?;
                let max_dist = r.r_f32()?;
                let base_volume = r.r_f32()?;
                let game_type = r.r_u32()?;
                Ok(Self { min_dist, max_dist, base_volume, game_type, max_ai_dist: max_dist })
            }
            OGG_COMMENT_VERSION => Ok(Self {
                min_dist: r.r_f32()?,
                max_dist: r.r_f32()?,
                base_volume: r.r_f32()?,
                game_type: r.r_u32()?,
                max_ai_dist: r.r_f32()?,
            }),
            other => Err(SoundError::CommentVersion(other)),
        }
    }

    /// The comment block of the current version: 24 bytes.
    #[must_use]
    pub fn to_comment(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(24);
        w.w_u32(OGG_COMMENT_VERSION);
        w.w_f32(self.min_dist);
        w.w_f32(self.max_dist);
        w.w_f32(self.base_volume);
        w.w_u32(self.game_type);
        w.w_f32(self.max_ai_dist);
        w.into_bytes()
    }

    /// Whether the engine accepts the distances (it stops on a sound that fails this).
    #[must_use]
    pub fn valid_distances(&self) -> bool {
        self.max_ai_dist >= 0.1 && self.max_dist >= 0.1
    }
}

const COMMENT_MAGIC: &[u8; 7] = b"\x03vorbis";

/// The parts of a Vorbis comment header packet, borrowed from it.
struct CommentPacket<'a> {
    vendor: &'a [u8],
    comments: Vec<&'a [u8]>,
}

fn parse_comment_packet(packet: &[u8]) -> Result<CommentPacket<'_>> {
    let body = packet
        .strip_prefix(COMMENT_MAGIC)
        .ok_or(SoundError::NotOgg("the second packet is not a Vorbis comment header"))?;
    let mut r = Reader::new(body);
    let cut = |_| SoundError::Damaged("the Vorbis comment header is cut short");
    let vendor_len = r.r_u32().map_err(cut)? as usize;
    let vendor = r.take(vendor_len).map_err(cut)?;
    let count = r.r_u32().map_err(cut)? as usize;
    // every comment takes at least its length field
    if count > r.elapsed() / 4 {
        return Err(SoundError::Damaged("the Vorbis comment header is cut short"));
    }
    let mut comments = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.r_u32().map_err(cut)? as usize;
        comments.push(r.take(len).map_err(cut)?);
    }
    Ok(CommentPacket { vendor, comments })
}

fn build_comment_packet(vendor: &[u8], comments: &[&[u8]]) -> Vec<u8> {
    let size = 7 + 8 + vendor.len() + comments.iter().map(|c| 4 + c.len()).sum::<usize>() + 1;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(COMMENT_MAGIC);
    out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor);
    out.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for comment in comments {
        out.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        out.extend_from_slice(comment);
    }
    // the framing bit
    out.push(1);
    out
}

fn headers_of(ogg: &[u8]) -> Result<(Vec<ogg::Page<'_>>, Headers)> {
    let pages = ogg::parse_pages(ogg)?;
    let headers = ogg::read_headers(&pages)?;
    if !headers.ident.starts_with(b"\x01vorbis") {
        return Err(SoundError::NotOgg("the first packet is not a Vorbis identification header"));
    }
    Ok((pages, headers))
}

/// The game's parameters of an `.ogg`. `None` when the file has no comment at all - the engine
/// then plays the sound with its defaults; a comment the game cannot read is an error.
pub fn read_ogg_params(ogg: &[u8]) -> Result<Option<OggParams>> {
    let (_, headers) = headers_of(ogg)?;
    let packet = parse_comment_packet(&headers.comment)?;
    packet.comments.first().map(|c| OggParams::from_comment(c)).transpose()
}

/// The same `.ogg` with other parameters. Only the comment header changes: its packet is built
/// again around the new block - the vendor string and any further comments stay - and the pages
/// that carried the comment and setup headers are written anew. Audio pages are copied; when
/// the number of header pages changes they are renumbered, with their checksums.
pub fn write_ogg_params(ogg: &[u8], params: &OggParams) -> Result<Vec<u8>> {
    let (pages, headers) = headers_of(ogg)?;
    if !headers.clean {
        return Err(SoundError::Layout("its header packets share pages with other packets"));
    }
    let old = parse_comment_packet(&headers.comment)?;
    let block = params.to_comment();
    let mut comments: Vec<&[u8]> = Vec::with_capacity(old.comments.len().max(1));
    comments.push(&block);
    comments.extend(old.comments.iter().skip(1));
    let packet = build_comment_packet(old.vendor, &comments);

    let first =
        pages.get(headers.first_page).ok_or(SoundError::Damaged("the Vorbis headers are incomplete"))?;
    let page_bytes = |p: &ogg::Page<'_>| ogg.get(p.start..p.start + p.len).unwrap_or(&[]);
    let first_sequence = ogg::sequence_of(page_bytes(first));

    let mut out = Vec::with_capacity(ogg.len() + packet.len());
    out.extend_from_slice(ogg.get(..first.start).unwrap_or(&[]));

    // the header pages: ours are replaced, another stream's stay where they are
    let new_count = ogg::write_pages(&[&packet, &headers.setup], headers.serial, first_sequence, &mut out);
    let mut old_count = 0u32;
    let mut cursor = first.start;
    for page in pages.iter().take(headers.last_page + 1).skip(headers.first_page) {
        if page.serial == headers.serial {
            old_count += 1;
        } else {
            out.extend_from_slice(page_bytes(page));
        }
        cursor = page.start + page.len;
    }

    // everything after: copied, or renumbered when the header pages changed in number
    if new_count == old_count {
        out.extend_from_slice(ogg.get(cursor..).unwrap_or(&[]));
    } else {
        for page in pages.iter().skip(headers.last_page + 1) {
            // junk between pages goes along
            out.extend_from_slice(ogg.get(cursor..page.start).unwrap_or(&[]));
            let bytes = page_bytes(page);
            if page.serial == headers.serial {
                let sequence = ogg::sequence_of(bytes).wrapping_sub(old_count).wrapping_add(new_count);
                ogg::write_renumbered(bytes, sequence, &mut out);
            } else {
                out.extend_from_slice(bytes);
            }
            cursor = page.start + page.len;
        }
        out.extend_from_slice(ogg.get(cursor..).unwrap_or(&[]));
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const VENDOR: &[u8] = b"Xiph.Org libVorbis I 20020717";

    /// An Ogg file with Vorbis-shaped headers and one "audio" page. The setup packet is not a
    /// real codebook, so this serves the container tests only.
    pub(crate) fn fake_ogg(vendor: &[u8], comments: &[&[u8]], setup_len: usize) -> Vec<u8> {
        let mut ident = b"\x01vorbis".to_vec();
        ident.extend_from_slice(&[0u8; 23]);
        let comment = build_comment_packet(vendor, comments);
        let mut setup = b"\x05vorbis".to_vec();
        setup.resize(setup_len, 0x5a);

        let mut file = Vec::new();
        ogg::write_pages(&[&ident], 0, 0, &mut file);
        let n = ogg::write_pages(&[&comment, &setup], 0, 1, &mut file);
        // an audio page with a granule position, the way libogg numbers it
        let mut page = Vec::new();
        ogg::write_pages(&[&[0x42u8; 100]], 0, 1 + n, &mut page);
        page[6..14].copy_from_slice(&44100u64.to_le_bytes());
        ogg::write_renumbered(&page, 1 + n, &mut file);
        file
    }

    #[test]
    fn every_version_of_the_block_is_read() {
        let p =
            OggParams { min_dist: 1.0, max_dist: 30.0, base_volume: 0.7, game_type: 5, max_ai_dist: 20.0 };
        let block = p.to_comment();
        assert_eq!(block.len(), 24);
        assert_eq!(&block[..4], &[3, 0, 0, 0]);
        assert_eq!(OggParams::from_comment(&block).unwrap(), p);

        let mut w = Writer::new();
        w.w_u32(1);
        w.w_f32(2.0);
        w.w_f32(40.0);
        w.w_u32(9);
        let v1 = OggParams::from_comment(w.bytes()).unwrap();
        assert_eq!(
            v1,
            OggParams { min_dist: 2.0, max_dist: 40.0, base_volume: 1.0, game_type: 9, max_ai_dist: 40.0 }
        );

        let mut w = Writer::new();
        w.w_u32(2);
        w.w_f32(2.0);
        w.w_f32(40.0);
        w.w_f32(0.5);
        w.w_u32(9);
        let v2 = OggParams::from_comment(w.bytes()).unwrap();
        assert_eq!((v2.base_volume, v2.max_ai_dist), (0.5, 40.0));

        assert_eq!(OggParams::from_comment(&[7, 0, 0, 0]), Err(SoundError::CommentVersion(7)));
        assert!(matches!(OggParams::from_comment(&block[..10]), Err(SoundError::Chunk(_))));
        assert!(p.valid_distances());
    }

    #[test]
    fn parameters_are_read_and_replaced_in_place() {
        let old =
            OggParams { min_dist: 1.0, max_dist: 30.0, base_volume: 1.0, game_type: 0, max_ai_dist: 30.0 };
        let file = fake_ogg(VENDOR, &[&old.to_comment(), b"TITLE=keep me"], 600);
        assert_eq!(read_ogg_params(&file).unwrap(), Some(old));

        let new = OggParams {
            min_dist: 3.0,
            max_dist: 90.0,
            base_volume: 1.2,
            game_type: 0x400,
            max_ai_dist: 45.0,
        };
        let out = write_ogg_params(&file, &new).unwrap();
        // same size, same pages: only the block and a checksum differ
        assert_eq!(out.len(), file.len());
        assert_eq!(read_ogg_params(&out).unwrap(), Some(new));

        let (pages, headers) = headers_of(&out).unwrap();
        let packet = parse_comment_packet(&headers.comment).unwrap();
        assert_eq!(packet.vendor, VENDOR);
        assert_eq!(packet.comments[1], b"TITLE=keep me");
        // the audio page is untouched
        let last = pages.last().unwrap();
        assert_eq!(last.granule, 44100);
        assert_eq!(&out[last.start..], &file[file.len() - last.len..]);
    }

    #[test]
    fn a_file_without_a_comment_gets_one_and_its_pages_are_renumbered() {
        // 255 segments, a page exactly: a comment packet of 240 bytes and a setup packet of 254
        // segments. The block makes the comment packet 268 bytes - one segment more, a page more.
        let file = fake_ogg(&[b'v'; 224], &[], 253 * 255 + 10);
        assert_eq!(read_ogg_params(&file).unwrap(), None);
        let before = ogg::parse_pages(&file).unwrap().len();

        let p =
            OggParams { min_dist: 1.0, max_dist: 10.0, base_volume: 1.0, game_type: 1, max_ai_dist: 10.0 };
        let out = write_ogg_params(&file, &p).unwrap();
        assert_eq!(read_ogg_params(&out).unwrap(), Some(p));

        let pages = ogg::parse_pages(&out).unwrap();
        assert_eq!(pages.len(), before + 1);
        for (n, page) in pages.iter().enumerate() {
            let bytes = &out[page.start..page.start + page.len];
            assert_eq!(ogg::sequence_of(bytes) as usize, n);
            let mut copy = bytes.to_vec();
            let stored = u32::from_le_bytes([copy[22], copy[23], copy[24], copy[25]]);
            copy[22..26].copy_from_slice(&[0; 4]);
            assert_eq!(ogg::crc(&copy), stored);
        }
        assert_eq!(pages.last().unwrap().granule, 44100);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(read_ogg_params(&[]).is_err());
        assert!(read_ogg_params(b"RIFF....WAVEfmt ").is_err());

        // a comment header that claims more comments than it has bytes
        let mut ident = b"\x01vorbis".to_vec();
        ident.extend_from_slice(&[0u8; 23]);
        let mut comment = COMMENT_MAGIC.to_vec();
        comment.extend_from_slice(&0u32.to_le_bytes());
        comment.extend_from_slice(&u32::MAX.to_le_bytes());
        let mut file = Vec::new();
        ogg::write_pages(&[&ident], 0, 0, &mut file);
        ogg::write_pages(&[&comment, b"\x05vorbis"], 0, 1, &mut file);
        assert!(matches!(read_ogg_params(&file), Err(SoundError::Damaged(_))));

        // not Vorbis inside the Ogg
        let mut file = Vec::new();
        ogg::write_pages(&[b"OpusHead"], 0, 0, &mut file);
        ogg::write_pages(&[b"OpusTags", b"x"], 0, 1, &mut file);
        assert!(matches!(read_ogg_params(&file), Err(SoundError::NotOgg(_))));
    }
}
