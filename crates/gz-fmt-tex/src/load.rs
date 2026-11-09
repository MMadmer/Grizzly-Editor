//! One door for every picture file the editor takes.

use std::io::Cursor;

use image::{ColorType, ImageReader, Limits};

use crate::dds::read_dds_level;
use crate::tga::read_tga;
use crate::{Image, MAX_SIDE, Result, TexError};

/// A picture and whether its file carried an alpha channel.
///
/// The flag is what the image manager derives from the file, not from the pixels: a 32-bit
/// TGA, a four-channel PNG or BMP, and every DDS count as having alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub image: Image,
    pub has_alpha: bool,
}

fn is_tga_name(ext: &str) -> bool {
    ext.trim_start_matches('.').eq_ignore_ascii_case("tga")
}

fn through_image_crate(bytes: &[u8]) -> Result<Loaded> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| TexError::Decode(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| TexError::Decode(e.to_string()))?;
    // the manager's rule: exactly four channels in the file
    let has_alpha = matches!(decoded.color(), ColorType::Rgba8 | ColorType::Rgba16 | ColorType::Rgba32F);
    let rgba = decoded.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Ok(Loaded { image: Image::new(width, height, rgba.into_raw())?, has_alpha })
}

/// Reads a picture: DDS (the top level of the first layer), TGA, PNG, JPG or BMP.
///
/// `ext` is the file's extension when the caller knows it. A `.tga` goes to the editor's own
/// TGA reader, as in the image manager; without the hint the bytes decide, and TGA - which has
/// no signature - is tried last.
pub fn load_image(bytes: &[u8], ext: Option<&str>) -> Result<Loaded> {
    if ext.is_some_and(is_tga_name) {
        return read_tga(bytes);
    }
    if bytes.starts_with(b"DDS ") {
        return Ok(Loaded { image: read_dds_level(bytes, 0, 0)?, has_alpha: true });
    }
    match through_image_crate(bytes) {
        Ok(loaded) => Ok(loaded),
        Err(first) => read_tga(bytes).map_err(|_| first),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds::{DdsEncoding, encode_dds};
    use crate::tga::{TgaDepth, write_tga};

    fn sample() -> Image {
        Image::new(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 70]).unwrap()
    }

    /// A 2 x 1 BMP of 24 bits: blue-green-red rows, padded to four bytes.
    fn bmp() -> Vec<u8> {
        let mut out = b"BM".to_vec();
        out.extend_from_slice(&(54u32 + 8).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(&[30, 20, 10, 60, 50, 40, 0, 0]);
        out
    }

    #[test]
    fn each_kind_of_file_finds_its_reader() {
        let img = sample();
        let tga = write_tga(&img, TgaDepth::Bgra32).unwrap();
        let by_name = load_image(&tga, Some(".TGA")).unwrap();
        assert_eq!(by_name.image, img);
        assert!(by_name.has_alpha);
        // without the name the signature-less TGA is still found
        assert_eq!(load_image(&tga, None).unwrap().image, img);

        let dds = encode_dds(std::slice::from_ref(&img), DdsEncoding::Rgba8).unwrap();
        let from_dds = load_image(&dds, Some("dds")).unwrap();
        assert_eq!(from_dds.image, img);
        assert!(from_dds.has_alpha);

        let from_bmp = load_image(&bmp(), None).unwrap();
        assert_eq!(from_bmp.image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
        assert!(!from_bmp.has_alpha);
    }

    #[test]
    fn garbage_is_an_error_whatever_the_name() {
        assert!(load_image(&[], None).is_err());
        assert!(load_image(b"neither a picture nor anything else", None).is_err());
        assert!(load_image(b"neither a picture nor anything else", Some("png")).is_err());
        assert!(load_image(b"DDS ", None).is_err());
        assert!(load_image(&[0; 3], Some("tga")).is_err());
    }
}
