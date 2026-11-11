//! Textures of the game and the editor: pixels in, files out, and back.
//!
//! Every function takes bytes or pixels and returns values; nothing here opens a file. Pixel
//! work runs on the caller's rayon pool. Hostile input is an `Err`, never a panic; no side of a
//! picture may exceed [`MAX_SIDE`].
//!
//! **Pixels**
//! - [`Image`] - RGBA8, rows top-down. [`Loaded`] - an image and whether its file carried alpha.
//! - [`load_image`] - DDS, TGA, PNG, JPG or BMP bytes into a [`Loaded`] (top level only).
//! - [`read_tga`] / [`write_tga`] - 8, 24 and 32 bit, RLE on reading.
//! - [`resize`] - the filtered resize the game textures' mip levels are made with.
//! - [`mip_chain`] / [`mip_count`] - every level under an image.
//! - [`imf_resample`] - the engine's own resampler (thumbnails, the compliance check).
//!
//! **DDS**
//! - [`read_dds_info`] - the header: size, levels, layers, format.
//! - [`read_dds`] - every layer and level as RGBA8; [`read_dds_level`] - one of them.
//! - [`encode_dds`] - compresses given levels and writes the file; [`build_dds`] - makes the
//!   levels first. [`DdsEncoding`] names what can be written: DXT1, DXT1 with 1-bit alpha, DXT3,
//!   DXT5, BC4, BC5, RGBA, BGRA, RGB.
//!
//! **Descriptions (`.thm`)**
//! - [`TextureParams`] - `STextureParams`, with [`TextureParams::dds_request`] (what DDS a
//!   description asks for), [`TextureParams::for_new_texture`], [`TextureParams::apply_type_defaults`],
//!   [`TextureParams::memory_usage`].
//! - [`read_thm`] - any thumbnail file as a [`Thm`]; [`thm_kind`] - which one it is.
//! - [`read_texture_thm`] / [`write_texture_thm`], [`read_object_thm`] / [`write_object_thm`],
//!   [`read_group_thm`] / [`write_group_thm`]; the sound one is `gz_fmt_sound`'s, re-exported as
//!   [`SoundThm`], [`read_sound_thm`], [`write_sound_thm`].
//! - [`thumbnail_pixels`] - the 128 x 128 picture a thumbnail stores.
//!
//! **What the editor makes**
//! - [`make_game_texture`] - a source picture and its description into the game's `.dds`
//!   (and the `#.dds` of a bump map): [`GameTexture`].
//! - [`make_bump_textures`] - the bump pair alone.
//! - [`make_thumbnail_dds`] / [`valid_thumbnail_dds`] - a module's `thumbnail.dds`.
//! - [`make_screen_picture_dds`] - a screen layer's picture.
//! - [`texture_tile`] - the picture of an image file for a content browser tile.
//! - [`check_compliance`], [`apply_borders`], [`is_valid_size`] - the image manager's checks.
//! - [`cubemap::simplify`] - a cube strip's smaller copy (Cube Map \ Smaller Copy).

mod bc;
mod bump;
pub mod cubemap;
mod dds;
mod load;
mod make;
mod mip;
mod params;
mod resample;
mod tga;
mod thm;
mod tile;

pub use bump::{BumpTextures, make_bump_textures};
pub use dds::{
    Dds, DdsEncoding, DdsFormat, DdsInfo, DdsKind, PixelMasks, build_dds, encode_dds, read_dds,
    read_dds_info, read_dds_level,
};
pub use gz_fmt_sound::{SoundThm, read_sound_thm, write_sound_thm};
pub use load::{Loaded, load_image};
pub use make::{
    GameTexture, apply_borders, check_compliance, is_valid_size, make_game_texture, make_screen_picture_dds,
    make_thumbnail_dds, thumbnail_pixels, valid_thumbnail_dds,
};
pub use mip::{MipFilter, mip_chain, mip_count, resize};
pub use params::{
    BumpMode, DdsRequest, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams,
    TextureType,
};
pub use resample::{ImfFilter, imf_resample};
pub use tga::{TgaDepth, read_tga, write_tga};
pub use thm::{
    GroupThm, ObjectThm, THUMB_SIDE, TextureThm, Thm, ThmKind, read_group_thm, read_object_thm,
    read_texture_thm, read_thm, thm_kind, write_group_thm, write_object_thm, write_texture_thm,
};
pub use tile::{TextureTile, TileAlpha, texture_tile};

/// The longest side a picture may have.
pub const MAX_SIDE: u32 = 16384;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TexError {
    #[error("{0}")]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("the picture is {width} x {height}; a side must be from 1 to 16384 pixels")]
    BadSize { width: u64, height: u64 },
    #[error("the pixel data holds {got} bytes, {width} x {height} needs {need}")]
    BadPixels { width: u32, height: u32, need: usize, got: usize },
    #[error("the file is cut short: {need} bytes are missing")]
    Truncated { need: usize },
    #[error("this is not a {0} file")]
    NotA(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("the picture could not be read: {0}")]
    Decode(String),
    #[error("the thumbnail has version {0:#06x}, which this editor does not read")]
    ThmVersion(u16),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = TexError> = std::result::Result<T, E>;

/// RGBA8 pixels, rows top-down, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Checks the size against the pixels.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self> {
        let need = checked_len(width, height)?;
        if rgba.len() != need {
            return Err(TexError::BadPixels { width, height, need, got: rgba.len() });
        }
        Ok(Self { width, height, rgba })
    }

    /// A picture of one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self> {
        let need = checked_len(width, height)?;
        let mut data = vec![0u8; need];
        for px in data.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&rgba);
        }
        Ok(Self { width, height, rgba: data })
    }

    /// Whether the pixels fit the size; every function that takes an `Image` checks this first.
    pub fn check(&self) -> Result<()> {
        let need = checked_len(self.width, self.height)?;
        if self.rgba.len() != need {
            return Err(TexError::BadPixels {
                width: self.width,
                height: self.height,
                need,
                got: self.rgba.len(),
            });
        }
        Ok(())
    }

    /// Whether any pixel is less than opaque.
    #[must_use]
    pub fn has_transparency(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255)
    }
}

/// `width * height * 4`, refused when a side is zero or over [`MAX_SIDE`].
pub(crate) fn checked_len(width: u32, height: u32) -> Result<usize> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(TexError::BadSize { width: u64::from(width), height: u64::from(height) });
    }
    Ok(width as usize * height as usize * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_checks_its_size() {
        assert!(Image::new(2, 2, vec![0; 16]).is_ok());
        assert!(matches!(Image::new(2, 2, vec![0; 15]), Err(TexError::BadPixels { .. })));
        assert!(matches!(Image::new(0, 2, vec![]), Err(TexError::BadSize { .. })));
        assert!(matches!(Image::new(MAX_SIDE + 1, 1, vec![]), Err(TexError::BadSize { .. })));
        let img = Image::filled(3, 1, [1, 2, 3, 4]).unwrap();
        assert_eq!(img.rgba, [1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
        assert!(img.has_transparency());
    }
}
