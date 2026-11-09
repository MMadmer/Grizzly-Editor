//! Block compression: decoding BC1-BC7 into RGBA8, and the encoder front.

use rayon::prelude::*;

/// The block formats a DDS can hold.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6h { signed: bool },
    Bc7,
}

impl Block {
    pub(crate) fn bytes(self) -> usize {
        match self {
            Block::Bc1 | Block::Bc4 => 8,
            _ => 16,
        }
    }
}

/// Blocks along a side.
pub(crate) fn blocks(side: usize) -> usize {
    side.div_ceil(4)
}

/// Bytes a level takes in a block format.
pub(crate) fn level_bytes(block: Block, width: usize, height: usize) -> usize {
    blocks(width) * blocks(height) * block.bytes()
}

fn arr<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = src.len().min(N);
    out[..n].copy_from_slice(&src[..n]);
    out
}

fn expand565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u8;
    let g = ((c >> 5) & 63) as u8;
    let b = (c & 31) as u8;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// A colour block's sixteen pixels. `punch` is BC1: the block whose first colour is not the
/// greater one has three colours and a transparent fourth. BC2 and BC3 always have four.
fn colour_block(block: &[u8; 8], punch: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let a = expand565(c0);
    let b = expand565(c1);
    let mut palette = [[0u8, 0, 0, 255]; 4];
    palette[0][..3].copy_from_slice(&a);
    palette[1][..3].copy_from_slice(&b);
    if c0 > c1 || !punch {
        for i in 0..3 {
            palette[2][i] = ((2 * u16::from(a[i]) + u16::from(b[i])) / 3) as u8;
            palette[3][i] = ((2 * u16::from(b[i]) + u16::from(a[i])) / 3) as u8;
        }
    } else {
        for i in 0..3 {
            palette[2][i] = ((u16::from(a[i]) + u16::from(b[i])) / 2) as u8;
        }
        palette[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, px) in out.iter_mut().enumerate() {
        *px = palette[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

/// The sixteen values of an interpolated alpha block (BC3 alpha, BC4, each half of BC5).
fn alpha_block(block: &[u8; 8]) -> [u8; 16] {
    let a0 = u16::from(block[0]);
    let a1 = u16::from(block[1]);
    let mut palette = [0u8; 8];
    palette[0] = block[0];
    palette[1] = block[1];
    if a0 > a1 {
        for i in 1..7u16 {
            palette[usize::from(i) + 1] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            palette[usize::from(i) + 1] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    let bits = u64::from_le_bytes(arr::<8>(block)) >> 16;
    let mut out = [0u8; 16];
    for (i, v) in out.iter_mut().enumerate() {
        *v = palette[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

fn decode_block(kind: Block, block: &[u8]) -> [[u8; 4]; 16] {
    match kind {
        Block::Bc1 => colour_block(&arr(block), true),
        Block::Bc2 => {
            let mut px = colour_block(&arr(block.get(8..).unwrap_or(&[])), false);
            let bits = u64::from_le_bytes(arr::<8>(block));
            for (i, p) in px.iter_mut().enumerate() {
                let a = ((bits >> (4 * i)) & 15) as u8;
                p[3] = a | (a << 4);
            }
            px
        }
        Block::Bc3 => {
            let mut px = colour_block(&arr(block.get(8..).unwrap_or(&[])), false);
            let alpha = alpha_block(&arr(block));
            for (p, a) in px.iter_mut().zip(alpha) {
                p[3] = a;
            }
            px
        }
        Block::Bc4 => {
            let v = alpha_block(&arr(block));
            let mut px = [[0u8, 0, 0, 255]; 16];
            for (p, v) in px.iter_mut().zip(v) {
                *p = [v, v, v, 255];
            }
            px
        }
        Block::Bc5 => {
            let x = alpha_block(&arr(block));
            let y = alpha_block(&arr(block.get(8..).unwrap_or(&[])));
            let mut px = [[0u8, 0, 0, 255]; 16];
            for ((p, x), y) in px.iter_mut().zip(x).zip(y) {
                *p = [x, y, 0, 255];
            }
            px
        }
        Block::Bc6h { signed } => {
            let data: [u8; 16] = arr(block);
            let mut rgb = [0f32; 48];
            bcdec_rs::bc6h_float(&data, &mut rgb, 12, signed);
            let mut px = [[0u8, 0, 0, 255]; 16];
            for (p, c) in px.iter_mut().zip(rgb.chunks_exact(3)) {
                for (d, &s) in p.iter_mut().zip(c) {
                    // high dynamic range is shown clamped, not tone mapped
                    *d = (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
            px
        }
        Block::Bc7 => {
            let data: [u8; 16] = arr(block);
            let mut flat = [0u8; 64];
            bcdec_rs::bc7(&data, &mut flat, 16);
            let mut px = [[0u8; 4]; 16];
            for (p, c) in px.iter_mut().zip(flat.chunks_exact(4)) {
                p.copy_from_slice(c);
            }
            px
        }
    }
}

/// Decodes one level into `width * height` RGBA8 pixels. `data` holds at least
/// [`level_bytes`] bytes; whatever is short of that decodes as zero blocks.
pub(crate) fn decode_level(kind: Block, data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    if width == 0 || height == 0 {
        return out;
    }
    let row_bytes = blocks(width) * kind.bytes();
    let bs = kind.bytes();
    out.par_chunks_mut(width * 4 * 4).enumerate().for_each(|(by, rows)| {
        let line = data.get(by * row_bytes..).unwrap_or(&[]);
        for bx in 0..blocks(width) {
            let block = line.get(bx * bs..(bx + 1) * bs).unwrap_or(&[]);
            let px = decode_block(kind, block);
            let cols = (width - bx * 4).min(4);
            for (py, row) in rows.chunks_exact_mut(width * 4).enumerate() {
                let dst = &mut row[bx * 16..bx * 16 + cols * 4];
                for (d, s) in dst.chunks_exact_mut(4).zip(&px[py * 4..py * 4 + cols]) {
                    d.copy_from_slice(s);
                }
            }
        }
    });
    out
}

/// What happens to alpha on the way into the compressor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AlphaRule {
    /// As it is.
    Keep,
    /// Every pixel opaque: DXT1 without alpha.
    Opaque,
    /// Over 127 is opaque, the rest transparent: DXT1 with 1-bit alpha.
    Binary,
}

/// Compresses one level. Blocks at the right and bottom edges of a size that is no multiple
/// of four are fitted to the pixels they really have.
pub(crate) fn encode_level(format: texpresso::Format, alpha: AlphaRule, rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let bs = format.block_size();
    let wide = blocks(width);
    let mut out = vec![0u8; wide * blocks(height) * bs];
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return out;
    }
    // what the editor's compressor is set to: cluster fit, every channel weighed alike
    let params = texpresso::Params {
        algorithm: texpresso::Algorithm::ClusterFit,
        weights: texpresso::COLOUR_WEIGHTS_UNIFORM,
        weigh_colour_by_alpha: false,
    };
    out.par_chunks_mut(wide * bs).enumerate().for_each(|(by, line)| {
        for (bx, block) in line.chunks_exact_mut(bs).enumerate() {
            let mut px = [[0u8; 4]; 16];
            let mut mask = 0u32;
            for py in 0..4 {
                let sy = by * 4 + py;
                if sy >= height {
                    break;
                }
                for qx in 0..4 {
                    let sx = bx * 4 + qx;
                    if sx >= width {
                        break;
                    }
                    let at = (sy * width + sx) * 4;
                    let mut p: [u8; 4] = arr(&rgba[at..at + 4]);
                    match alpha {
                        AlphaRule::Keep => {}
                        AlphaRule::Opaque => p[3] = 255,
                        AlphaRule::Binary => p[3] = if p[3] > 127 { 255 } else { 0 },
                    }
                    px[py * 4 + qx] = p;
                    mask |= 1 << (py * 4 + qx);
                }
            }
            format.compress_block_masked(px, mask, params, block);
        }
    });
    out
}

/// Decodes the first level of a DXT5 image the way the bump generator's decompressor does:
/// colours widened by shifting alone, the three-colour mode honoured, whole blocks only (the
/// rest stays zero). The error texture of a bump map is the difference to this picture, so the
/// arithmetic is kept as it is.
pub(crate) fn nv_decode_dxt5(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    let (xblocks, yblocks) = (width / 4, height / 4);
    if xblocks == 0 || yblocks == 0 {
        return out;
    }
    // the file's blocks cover the rounded-up size
    let row_bytes = blocks(width) * 16;
    out.par_chunks_mut(width * 16).take(yblocks).enumerate().for_each(|(by, rows)| {
        let line = data.get(by * row_bytes..).unwrap_or(&[]);
        for bx in 0..xblocks {
            let block: [u8; 16] = arr(line.get(bx * 16..(bx + 1) * 16).unwrap_or(&[]));
            let c0 = u16::from_le_bytes([block[8], block[9]]);
            let c1 = u16::from_le_bytes([block[10], block[11]]);
            let wide = |c: u16| [(((c >> 11) & 31) << 3) as u8, (((c >> 5) & 63) << 2) as u8, ((c & 31) << 3) as u8];
            let (a, b) = (wide(c0), wide(c1));
            let mut palette = [[0u8; 3]; 4];
            palette[0] = a;
            palette[1] = b;
            if c0 > c1 {
                for i in 0..3 {
                    palette[2][i] = ((u16::from(a[i]) * 2 + u16::from(b[i])) / 3) as u8;
                    palette[3][i] = ((u16::from(a[i]) + u16::from(b[i]) * 2) / 3) as u8;
                }
            } else {
                for i in 0..3 {
                    palette[2][i] = ((u16::from(a[i]) + u16::from(b[i])) / 2) as u8;
                }
                palette[3] = [0x00, 0xff, 0xff];
            }
            let colour_bits = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);

            let (a0, a1) = (u16::from(block[0]), u16::from(block[1]));
            let mut alphas = [0u16; 8];
            alphas[0] = a0;
            alphas[1] = a1;
            if a0 > a1 {
                for i in 1..7u16 {
                    alphas[usize::from(i) + 1] = ((7 - i) * a0 + i * a1) / 7;
                }
            } else {
                for i in 1..5u16 {
                    alphas[usize::from(i) + 1] = ((5 - i) * a0 + i * a1) / 5;
                }
                alphas[6] = 0;
                alphas[7] = 255;
            }
            let alpha_bits = u64::from_le_bytes(arr::<8>(&block)) >> 16;

            for (py, row) in rows.chunks_exact_mut(width * 4).enumerate() {
                for qx in 0..4 {
                    let i = py * 4 + qx;
                    let rgb = palette[((colour_bits >> (2 * i)) & 3) as usize];
                    let alpha = alphas[((alpha_bits >> (3 * i)) & 7) as usize] as u8;
                    let at = (bx * 4 + qx) * 4;
                    row[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
                }
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Red and blue endpoints, one row of each palette entry.
    fn dxt1_block() -> [u8; 8] {
        // c0 = 0xf800 (red) > c1 = 0x001f (blue): four colours
        [0x00, 0xf8, 0x1f, 0x00, 0b0000_0000, 0b0101_0101, 0b1010_1010, 0b1111_1111]
    }

    #[test]
    fn a_hand_made_dxt1_block_decodes() {
        let px = decode_level(Block::Bc1, &dxt1_block(), 4, 4);
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&px[16..20], &[0, 0, 255, 255]);
        // two thirds red, one third blue
        assert_eq!(&px[32..36], &[170, 0, 85, 255]);
        assert_eq!(&px[48..52], &[85, 0, 170, 255]);
        // every pixel of a row is the same entry
        assert_eq!(&px[4..8], &px[0..4]);
    }

    #[test]
    fn a_three_colour_block_is_transparent_in_dxt1_only() {
        // c0 = blue < c1 = red: three colours, the fourth transparent black
        let block = [0x1f, 0x00, 0x00, 0xf8, 0xff, 0xff, 0xaa, 0xaa];
        let px = decode_level(Block::Bc1, &block, 4, 4);
        assert_eq!(&px[0..4], &[0, 0, 0, 0]);
        assert_eq!(&px[32..36], &[127, 0, 127, 255]);

        let mut dxt5 = [0u8; 16];
        dxt5[0] = 200;
        dxt5[1] = 200;
        dxt5[8..].copy_from_slice(&block);
        let px = decode_level(Block::Bc3, &dxt5, 4, 4);
        // four colours whatever the order, alpha from its own block
        assert_eq!(&px[0..4], &[170, 0, 85, 200]);
    }

    #[test]
    fn explicit_and_interpolated_alpha_decode() {
        let mut dxt3 = [0u8; 16];
        dxt3[0] = 0xf0; // first pixel alpha 0, second 15
        dxt3[8..].copy_from_slice(&dxt1_block());
        let px = decode_level(Block::Bc2, &dxt3, 4, 4);
        assert_eq!(px[3], 0);
        assert_eq!(px[7], 255);

        // a0 = 255 > a1 = 0, indices 0, 1, 2, 3 for the first four pixels
        let bc4 = [255, 0, 0b1000_1000, 0b0000_0110, 0, 0, 0, 0];
        let px = decode_level(Block::Bc4, &bc4, 4, 4);
        assert_eq!(px[0], 255);
        assert_eq!(px[4], 0);
        assert_eq!(px[8], 218); // (6 * 255 + 0) / 7
        assert_eq!(px[12], 182); // (5 * 255 + 0) / 7
        assert_eq!(&px[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn sizes_that_are_no_multiple_of_four_are_cropped() {
        let px = decode_level(Block::Bc1, &dxt1_block(), 3, 2);
        assert_eq!(px.len(), 3 * 2 * 4);
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&px[12..16], &[0, 0, 255, 255]);
        // short data decodes as zero blocks instead of failing
        let px = decode_level(Block::Bc7, &[1, 2, 3], 5, 5);
        assert_eq!(px.len(), 100);
        assert_eq!(level_bytes(Block::Bc3, 5, 5), 64);
        assert_eq!(level_bytes(Block::Bc1, 1, 1), 8);
    }

    #[test]
    fn encoding_keeps_flat_colours_and_alpha_rules() {
        let mut rgba = vec![0u8; 8 * 4 * 4];
        for p in rgba.chunks_exact_mut(4) {
            p.copy_from_slice(&[255, 0, 0, 100]);
        }
        let opaque = encode_level(texpresso::Format::Bc1, AlphaRule::Opaque, &rgba, 8, 4);
        assert_eq!(opaque.len(), 16);
        let px = decode_level(Block::Bc1, &opaque, 8, 4);
        assert!(px.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));

        let binary = encode_level(texpresso::Format::Bc1, AlphaRule::Binary, &rgba, 8, 4);
        let px = decode_level(Block::Bc1, &binary, 8, 4);
        assert!(px.chunks_exact(4).all(|p| p[3] == 0));

        let dxt5 = encode_level(texpresso::Format::Bc3, AlphaRule::Keep, &rgba, 8, 4);
        assert_eq!(dxt5.len(), 32);
        let px = decode_level(Block::Bc3, &dxt5, 8, 4);
        assert!(px.chunks_exact(4).all(|p| p == [255, 0, 0, 100]));
        // the bump generator's decoder widens 5 bits by shifting: 31 becomes 248
        let nv = nv_decode_dxt5(&dxt5, 8, 4);
        assert!(nv.chunks_exact(4).all(|p| p == [248, 0, 0, 100]));
    }
}
