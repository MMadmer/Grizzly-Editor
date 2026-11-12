//! The editor's icon file (`assets/grizzly-editor-icon.ico`): its images, and the Windows resource
//! that carries it inside an executable.
//!
//! [`images`] reads an `.ico`; [`icon_res`] turns one into a compiled resource file (`.res`: one
//! `RT_ICON` per image and the `RT_GROUP_ICON` that lists them) the linker takes as an input;
//! [`embed`] is what a binary crate's build script calls. Nothing here runs a tool: the resource
//! is written from the icon's bytes at build time, so no generated binary is kept in the tree.

use std::path::Path;

/// What is wrong with an icon file.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IcoError {
    #[error("the icon file is cut short")]
    Short,
    #[error("the file is not an icon (its header is not the one of an .ico)")]
    NotIcon,
    #[error("the icon file holds no image")]
    Empty,
    #[error("image {0} of the icon file lies outside the file")]
    Outside(usize),
}

/// One image of an icon file, as its directory describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcoImage<'a> {
    /// The side in pixels (an entry's 0 means 256).
    pub width: u32,
    pub height: u32,
    pub color_count: u8,
    pub planes: u16,
    pub bit_count: u16,
    /// The image itself: a PNG or a DIB, exactly as stored.
    pub data: &'a [u8],
}

impl IcoImage<'_> {
    /// Whether the image is stored as a PNG.
    #[must_use]
    pub fn is_png(&self) -> bool {
        self.data.starts_with(b"\x89PNG\r\n\x1a\n")
    }
}

fn le16(b: &[u8], at: usize) -> Result<u16, IcoError> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).ok_or(IcoError::Short)
}

fn le32(b: &[u8], at: usize) -> Result<u32, IcoError> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).ok_or(IcoError::Short)
}

/// The images of an `.ico` file, in the order of its directory.
pub fn images(ico: &[u8]) -> Result<Vec<IcoImage<'_>>, IcoError> {
    if le16(ico, 0)? != 0 || le16(ico, 2)? != 1 {
        return Err(IcoError::NotIcon);
    }
    let count = usize::from(le16(ico, 4)?);
    if count == 0 {
        return Err(IcoError::Empty);
    }
    if ico.len() < 6 + count * 16 {
        return Err(IcoError::Short);
    }
    (0..count)
        .map(|i| {
            let at = 6 + i * 16;
            let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
            let (size, offset) = (le32(ico, at + 8)? as usize, le32(ico, at + 12)? as usize);
            let data =
                offset.checked_add(size).and_then(|end| ico.get(offset..end)).ok_or(IcoError::Outside(i))?;
            Ok(IcoImage {
                width: side(ico[at]),
                height: side(ico[at + 1]),
                color_count: ico[at + 2],
                planes: le16(ico, at + 4)?,
                bit_count: le16(ico, at + 6)?,
                data,
            })
        })
        .collect()
}

/// The largest image of an icon (the first of the largest side).
#[must_use]
pub fn largest<'a>(images: &[IcoImage<'a>]) -> Option<IcoImage<'a>> {
    images.iter().copied().fold(None, |best, im| match best {
        Some(b) if b.width >= im.width => Some(b),
        _ => Some(im),
    })
}

/// The image to show at `side` pixels: the smallest one at least that large, else the largest.
#[must_use]
pub fn for_side<'a>(images: &[IcoImage<'a>], side: u32) -> Option<IcoImage<'a>> {
    images.iter().copied().filter(|im| im.width >= side).min_by_key(|im| im.width).or_else(|| largest(images))
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// The language of the resources: English (United States), what the resource compiler writes.
const LANGUAGE: u16 = 0x0409;
/// `MOVEABLE | DISCARDABLE`, and `PURE` besides for the group, as the resource compiler sets them.
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;
/// The group's id: the one icon of the executable, the one Explorer shows.
pub const GROUP_ID: u16 = 1;

/// One resource of a `.res` file with a numeric type and a numeric name.
fn put_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, name] {
        out.extend_from_slice(&u16::to_le_bytes(v));
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANGUAGE.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// The compiled resource file of an icon: every image as `RT_ICON` 1..n and the group
/// [`GROUP_ID`] over them - what `ICON` in an `.rc` file compiles to.
pub fn icon_res(ico: &[u8]) -> Result<Vec<u8>, IcoError> {
    let images = images(ico)?;
    let mut out = Vec::with_capacity(ico.len() + 64 + images.len() * 48);
    // a .res file opens with an empty resource that marks it as 32-bit
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    out.extend_from_slice(&[0; 16]);
    let mut group = Vec::with_capacity(6 + images.len() * 14);
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&u16::try_from(images.len()).unwrap_or(u16::MAX).to_le_bytes());
    for (id, im) in (1u16..).zip(&images) {
        put_resource(&mut out, RT_ICON, id, ICON_FLAGS, im.data);
        let side = |v: u32| if v >= 256 { 0 } else { v as u8 };
        group.extend_from_slice(&[side(im.width), side(im.height), im.color_count, 0]);
        group.extend_from_slice(&im.planes.max(1).to_le_bytes());
        group.extend_from_slice(&im.bit_count.to_le_bytes());
        group.extend_from_slice(&u32::try_from(im.data.len()).unwrap_or(u32::MAX).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    put_resource(&mut out, RT_GROUP_ICON, GROUP_ID, GROUP_FLAGS, &group);
    Ok(out)
}

/// For a binary crate's build script: compiles `ico` into `<OUT_DIR>/icon.res` and hands it to the
/// linker of the crate's executables. Nothing happens for a target other than Windows.
pub fn embed(ico: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", ico.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let bytes = std::fs::read(ico).map_err(|e| format!("{}: {e}", ico.display()))?;
    let res = icon_res(&bytes).map_err(|e| format!("{}: {e}", ico.display()))?;
    let out = std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set: run this from a build script")?;
    let path = Path::new(&out).join("icon.res");
    std::fs::write(&path, res).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("cargo:rustc-link-arg-bins={}", path.display());
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// An icon of two images with the given sides and contents.
    fn ico(parts: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0];
        out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
        let mut offset = 6 + parts.len() * 16;
        for (side, data) in parts {
            out.extend_from_slice(&[*side, *side, 0, 0, 0, 0, 32, 0]);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in parts {
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn reads_the_directory() {
        let file = ico(&[(16, b"small"), (0, b"\x89PNG\r\n\x1a\nbig")]);
        let all = images(&file).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].width, all[0].data), (16, &b"small"[..]));
        assert_eq!(all[1].width, 256);
        assert!(all[1].is_png() && !all[0].is_png());
        assert_eq!(largest(&all).unwrap().width, 256);
        assert_eq!(for_side(&all, 8).unwrap().width, 16);
        assert_eq!(for_side(&all, 17).unwrap().width, 256);
        assert_eq!(for_side(&all, 512).unwrap().width, 256);
    }

    #[test]
    fn refuses_what_is_no_icon() {
        assert_eq!(images(b"\0\0\x01"), Err(IcoError::Short));
        assert_eq!(images(b"\0\0\x02\0\x01\0"), Err(IcoError::NotIcon));
        assert_eq!(images(b"\0\0\x01\0\0\0"), Err(IcoError::Empty));
        let mut file = ico(&[(16, b"small")]);
        file.truncate(file.len() - 1);
        assert_eq!(images(&file), Err(IcoError::Outside(0)));
    }

    #[test]
    fn the_resource_file() {
        let file = ico(&[(16, b"abc"), (0, b"defgh")]);
        let res = icon_res(&file).unwrap();
        // the empty opening resource, then RT_ICON 1 with its data padded to four bytes
        assert_eq!(&res[..8], &[0, 0, 0, 0, 32, 0, 0, 0]);
        assert_eq!(le32(&res, 32).unwrap(), 3);
        assert_eq!(&res[40..48], &[0xFF, 0xFF, 3, 0, 0xFF, 0xFF, 1, 0]);
        assert_eq!(le16(&res, 52).unwrap(), ICON_FLAGS);
        assert_eq!(&res[64..68], b"abc\0");
        // RT_ICON 2, then the group naming both by id with the 256 side as 0
        assert_eq!(&res[68 + 8..68 + 16], &[0xFF, 0xFF, 3, 0, 0xFF, 0xFF, 2, 0]);
        assert_eq!(&res[68 + 32..68 + 37], b"defgh");
        let g = 68 + 40;
        assert_eq!(le32(&res, g).unwrap(), 6 + 2 * 14);
        assert_eq!(&res[g + 8..g + 16], &[0xFF, 0xFF, 14, 0, 0xFF, 0xFF, 1, 0]);
        let dir = &res[g + 32..];
        assert_eq!(&dir[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(&dir[6..10], &[16, 16, 0, 0]);
        assert_eq!(le16(dir, 10).unwrap(), 1);
        assert_eq!(le32(dir, 14).unwrap(), 3);
        assert_eq!(le16(dir, 18).unwrap(), 1);
        assert_eq!(dir[20], 0);
        assert_eq!(le16(dir, 32).unwrap(), 2);
        assert_eq!(res.len() % 4, 0);
    }
}
