//! TGA: the format of the texture sources.

use crate::load::Loaded;
use crate::{Image, Result, TexError, checked_len};

const HEADER: usize = 18;

/// What [`write_tga`] writes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TgaDepth {
    /// 8 bits of grey, taken from the red channel.
    Grey8,
    /// 24 bits, no alpha.
    Bgr24,
    /// 32 bits with alpha.
    Bgra32,
}

fn five(v: u16) -> u8 {
    let v = (v & 31) as u8;
    (v << 3) | (v >> 2)
}

/// One stored pixel as RGBA. `bytes` is 1 to 4 bytes wide.
fn pixel(raw: &[u8], bytes: usize, grey: bool, alpha_bits: u8) -> [u8; 4] {
    let at = |i: usize| raw.get(i).copied().unwrap_or(0);
    match (bytes, grey) {
        (1, _) => [at(0), at(0), at(0), 255],
        (2, true) => [at(0), at(0), at(0), at(1)],
        (2, false) => {
            let v = u16::from_le_bytes([at(0), at(1)]);
            let a = if alpha_bits != 0 && v & 0x8000 == 0 { 0 } else { 255 };
            [five(v >> 10), five(v >> 5), five(v), a]
        }
        (3, _) => [at(2), at(1), at(0), 255],
        _ => [at(2), at(1), at(0), at(3)],
    }
}

/// Reads a TGA: true colour, grey or colour-mapped, 8 to 32 bits, plain or run-length encoded.
///
/// The image manager itself takes true colour of 24 or 32 bits only (`CImage::LoadTGA`); for
/// those the result is its result, and the picture has alpha when it has 32 bits.
pub fn read_tga(bytes: &[u8]) -> Result<Loaded> {
    let head = bytes.get(..HEADER).ok_or(TexError::Truncated { need: HEADER.saturating_sub(bytes.len()) })?;
    let id_len = usize::from(head[0]);
    let image_type = head[2];
    let map_len = usize::from(u16::from_le_bytes([head[5], head[6]]));
    let map_entry = usize::from(head[7]).div_ceil(8);
    let width = u32::from(u16::from_le_bytes([head[12], head[13]]));
    let height = u32::from(u16::from_le_bytes([head[14], head[15]]));
    let bits = head[16];
    let desc = head[17];

    let (mapped, grey, rle) = match image_type {
        1 => (true, false, false),
        2 => (false, false, false),
        3 => (false, true, false),
        9 => (true, false, true),
        10 => (false, false, true),
        11 => (false, true, true),
        other => return Err(TexError::Unsupported(format!("TGA image type {other}"))),
    };
    let px_bytes = match (bits, mapped, grey) {
        (8, true, _) => 1,
        (8 | 16, false, true) => usize::from(bits / 8),
        (15 | 16, false, false) => 2,
        (24, false, false) => 3,
        (32, false, false) => 4,
        _ => return Err(TexError::Unsupported(format!("TGA pixels of {bits} bits"))),
    };
    let len = checked_len(width, height)?;
    let pixels = width as usize * height as usize;

    let map_at = HEADER + id_len;
    let data_at = map_at + map_len * map_entry;
    let palette = if mapped {
        if !matches!(map_entry, 2..=4) {
            return Err(TexError::Unsupported(format!("a TGA colour map of {} bits", head[7])));
        }
        bytes.get(map_at..data_at).ok_or(TexError::Truncated { need: data_at.saturating_sub(bytes.len()) })?
    } else {
        &[]
    };
    let data = bytes.get(data_at..).ok_or(TexError::Truncated { need: data_at.saturating_sub(bytes.len()) })?;

    // the file must be able to hold the picture before memory is taken for it: a plain file has
    // every pixel, a packed one at most 128 pixels for each packet of a count and one pixel
    let enough = if rle { data.len() / (1 + px_bytes) * 128 >= pixels } else { data.len() / px_bytes >= pixels };
    if !enough {
        return Err(TexError::Truncated { need: (pixels * px_bytes).saturating_sub(data.len()) });
    }

    let alpha_bits = desc & 0x0f;
    let decode = |raw: &[u8]| -> [u8; 4] {
        if mapped {
            let entry = usize::from(raw.first().copied().unwrap_or(0)) * map_entry;
            palette.get(entry..entry + map_entry).map_or([0, 0, 0, 255], |e| pixel(e, map_entry, false, alpha_bits))
        } else {
            pixel(raw, px_bytes, grey, alpha_bits)
        }
    };

    // oracle: the loader picks the row order by the right-to-left bit and then flips by the
    // top-to-bottom bit, which is right for the two origins every tool writes (bottom-left,
    // top-left) and upside down for the right-hand ones; kept
    let flip_rows = (desc & 0x10 == 0) != (desc & 0x20 != 0);
    let mirror = desc & 0x10 != 0;
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; len];
    let mut put = |index: usize, px: [u8; 4]| {
        let (x, y) = (index % w, index / w);
        let row = if flip_rows { h - 1 - y } else { y };
        let col = if mirror { w - 1 - x } else { x };
        if let Some(dst) = rgba.get_mut((row * w + col) * 4..(row * w + col) * 4 + 4) {
            dst.copy_from_slice(&px);
        }
    };

    if rle {
        let mut at = 0usize;
        let mut index = 0usize;
        let cut = TexError::Truncated { need: px_bytes };
        while index < pixels {
            let packet = *data.get(at).ok_or(cut.clone())?;
            at += 1;
            let count = usize::from(packet & 0x7f) + 1;
            if packet & 0x80 != 0 {
                let px = decode(data.get(at..at + px_bytes).ok_or(cut.clone())?);
                at += px_bytes;
                for _ in 0..count.min(pixels - index) {
                    put(index, px);
                    index += 1;
                }
            } else {
                for _ in 0..count.min(pixels - index) {
                    put(index, decode(data.get(at..at + px_bytes).ok_or(cut.clone())?));
                    at += px_bytes;
                    index += 1;
                }
            }
        }
    } else {
        for (index, raw) in data.chunks_exact(px_bytes).take(pixels).enumerate() {
            put(index, decode(raw));
        }
    }

    let has_alpha = if mapped {
        map_entry == 4
    } else if grey {
        px_bytes == 2
    } else {
        px_bytes == 4 || (bits == 16 && alpha_bits != 0)
    };
    Ok(Loaded { image: Image { width, height, rgba }, has_alpha })
}

/// Writes a TGA the way the editor does (`TGAdesc::maketga`): uncompressed, rows top-down.
///
/// oracle: the editor pads each 24-bit row to four bytes, which no reader - its own included -
/// expects; the rows are written without padding here.
pub fn write_tga(image: &Image, depth: TgaDepth) -> Result<Vec<u8>> {
    image.check()?;
    let (image_type, bits, desc, px_bytes) = match depth {
        TgaDepth::Grey8 => (3u8, 8u8, 0x20u8, 1usize),
        TgaDepth::Bgr24 => (2, 24, 0x20, 3),
        TgaDepth::Bgra32 => (2, 32, 0x2f, 4),
    };
    let mut out = Vec::with_capacity(HEADER + image.rgba.len() / 4 * px_bytes);
    out.extend_from_slice(&[0, 0, image_type, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&(image.width as u16).to_le_bytes());
    out.extend_from_slice(&(image.height as u16).to_le_bytes());
    out.push(bits);
    out.push(desc);
    for p in image.rgba.chunks_exact(4) {
        match depth {
            TgaDepth::Grey8 => out.push(p[0]),
            TgaDepth::Bgr24 => out.extend_from_slice(&[p[2], p[1], p[0]]),
            TgaDepth::Bgra32 => out.extend_from_slice(&[p[2], p[1], p[0], p[3]]),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        // 2 x 2: red, green / blue, white with half alpha
        Image::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128]).unwrap()
    }

    fn header(image_type: u8, w: u16, h: u16, bits: u8, desc: u8) -> Vec<u8> {
        let mut out = vec![0, 0, image_type, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&[bits, desc]);
        out
    }

    #[test]
    fn what_is_written_is_read_back() {
        let img = sample();
        let tga = write_tga(&img, TgaDepth::Bgra32).unwrap();
        assert_eq!(tga.len(), 18 + 16);
        assert_eq!(&tga[..18], &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x2f]);
        // the first pixel is red, stored blue first
        assert_eq!(&tga[18..22], &[0, 0, 255, 255]);
        let back = read_tga(&tga).unwrap();
        assert_eq!(back.image, img);
        assert!(back.has_alpha);

        let tga = write_tga(&img, TgaDepth::Bgr24).unwrap();
        assert_eq!(tga.len(), 18 + 12);
        let back = read_tga(&tga).unwrap();
        assert!(!back.has_alpha);
        assert_eq!(&back.image.rgba[..12], &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
        assert_eq!(back.image.rgba[15], 255);

        let tga = write_tga(&img, TgaDepth::Grey8).unwrap();
        assert_eq!(tga.len(), 18 + 4);
        let back = read_tga(&tga).unwrap();
        assert_eq!(&back.image.rgba[..8], &[255, 255, 255, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn a_bottom_up_file_comes_out_top_down() {
        // origin bottom-left: the first stored row is the bottom one
        let mut tga = header(2, 1, 2, 24, 0);
        tga.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let img = read_tga(&tga).unwrap().image;
        assert_eq!(img.rgba, [6, 5, 4, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn run_length_packets_decode() {
        // 4 x 1, top-left: a run of three, then one literal
        let mut tga = header(10, 4, 1, 32, 0x28);
        tga.extend_from_slice(&[0x82, 10, 20, 30, 40, 0x00, 1, 2, 3, 4]);
        let img = read_tga(&tga).unwrap().image;
        assert_eq!(img.rgba, [30, 20, 10, 40, 30, 20, 10, 40, 30, 20, 10, 40, 3, 2, 1, 4]);

        // grey, packed
        let mut tga = header(11, 3, 1, 8, 0x20);
        tga.extend_from_slice(&[0x82, 9]);
        assert_eq!(read_tga(&tga).unwrap().image.rgba, [9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255]);
    }

    #[test]
    fn colour_mapped_and_16_bit_files_decode() {
        // two palette entries of 24 bits, one pixel of each
        let mut tga = vec![0, 1, 1, 0, 0, 2, 0, 24, 0, 0, 0, 0, 2, 0, 1, 0, 8, 0x20];
        tga.extend_from_slice(&[0, 0, 255, 255, 0, 0]);
        tga.extend_from_slice(&[1, 0]);
        let img = read_tga(&tga).unwrap().image;
        assert_eq!(img.rgba, [0, 0, 255, 255, 255, 0, 0, 255]);

        let mut tga = header(2, 1, 1, 16, 0x20);
        tga.extend_from_slice(&0x7c00u16.to_le_bytes());
        assert_eq!(read_tga(&tga).unwrap().image.rgba, [255, 0, 0, 255]);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(matches!(read_tga(&[0; 5]), Err(TexError::Truncated { .. })));
        assert!(matches!(read_tga(&header(7, 4, 4, 24, 0)), Err(TexError::Unsupported(_))));
        assert!(matches!(read_tga(&header(2, 4, 4, 12, 0)), Err(TexError::Unsupported(_))));
        assert!(matches!(read_tga(&header(2, 0, 4, 24, 0)), Err(TexError::BadSize { .. })));
        // a large size with no data behind it takes no memory
        assert!(matches!(read_tga(&header(2, 16384, 16384, 32, 0)), Err(TexError::Truncated { .. })));
        assert!(matches!(read_tga(&header(10, 16384, 16384, 32, 0)), Err(TexError::Truncated { .. })));
        assert!(matches!(read_tga(&header(2, 60000, 2, 32, 0)), Err(TexError::BadSize { .. })));
        // a packed file that ends inside a packet
        let mut tga = header(10, 4, 1, 32, 0x28);
        tga.extend_from_slice(&[0x83, 1, 2]);
        assert!(read_tga(&tga).is_err());
    }
}
