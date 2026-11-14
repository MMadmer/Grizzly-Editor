//! The editor's thumbnail files (`.thm`): one container, four kinds.
//!
//! A file is chunks: the version, the kind, for the kinds with a picture 128 x 128 pixels, and
//! what the kind itself keeps. The picture lies in the file as BGRA rows bottom-up, compressed;
//! the models hold it as RGBA rows top-down.

use gz_chunk::{Reader, Writer};
use gz_fmt_sound::{SoundError, SoundThm};

use crate::params::TextureParams;
use crate::{Result, TexError};

const THM_CHUNK_VERSION: u32 = 0x0810;
const THM_CHUNK_DATA: u32 = 0x0811;
const THM_CHUNK_TYPE: u32 = 0x0813;
const THM_CHUNK_OBJECTPARAM: u32 = 0x0816;
const THM_CHUNK_GROUPPARAM: u32 = 0x0001;

const THM_TEXTURE_VERSION: u16 = 0x0012;
const THM_OBJECT_VERSION: u16 = 0x0012;
const THM_SOUND_VERSION: u16 = 0x0014;
const THM_GROUP_VERSION: u16 = 0x0001;

const TYPE_OBJECT: u32 = 0;
const TYPE_TEXTURE: u32 = 1;
const TYPE_SOUND: u32 = 2;

/// The side of a thumbnail's picture.
pub const THUMB_SIDE: u32 = 128;
const THUMB_BYTES: usize = (THUMB_SIDE * THUMB_SIDE * 4) as usize;
const THUMB_ROW: usize = (THUMB_SIDE * 4) as usize;

/// Which thumbnail a file is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ThmKind {
    Texture,
    Object,
    Group,
    Sound,
}

/// A texture's description. The editor writes no picture into it; files of the X-Ray SDK editor
/// carry one, and it is kept.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureThm {
    /// 128 x 128 RGBA, rows top-down.
    pub pixels: Option<Vec<u8>>,
    pub params: TextureParams,
}

/// A library object's thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectThm {
    /// 128 x 128 RGBA, rows top-down.
    pub pixels: Vec<u8>,
    pub face_count: u32,
    pub vertex_count: u32,
}

/// A group's thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupThm {
    /// 128 x 128 RGBA, rows top-down.
    pub pixels: Vec<u8>,
    /// The library objects the group holds.
    pub objects: Vec<String>,
}

/// Any thumbnail file.
#[derive(Clone, Debug, PartialEq)]
pub enum Thm {
    Texture(TextureThm),
    Object(ObjectThm),
    Group(GroupThm),
    Sound(SoundThm),
}

fn version_of(file: &Reader<'_>) -> Result<u16> {
    Ok(file.need_chunk(THM_CHUNK_VERSION)?.reader().r_u16()?)
}

fn type_of(file: &Reader<'_>) -> Result<u32> {
    Ok(file.need_chunk(THM_CHUNK_TYPE)?.reader().r_u32()?)
}

/// Tells the kind from the version and type chunks. An object and a group share a type and
/// differ by version.
pub fn thm_kind(bytes: &[u8]) -> Result<ThmKind> {
    let file = Reader::new(bytes);
    let version = version_of(&file)?;
    match (type_of(&file)?, version) {
        (TYPE_TEXTURE, THM_TEXTURE_VERSION) => Ok(ThmKind::Texture),
        (TYPE_SOUND, THM_SOUND_VERSION) => Ok(ThmKind::Sound),
        (TYPE_OBJECT, THM_OBJECT_VERSION) => Ok(ThmKind::Object),
        (TYPE_OBJECT, THM_GROUP_VERSION) => Ok(ThmKind::Group),
        (TYPE_OBJECT | TYPE_TEXTURE | TYPE_SOUND, _) => Err(TexError::ThmVersion(version)),
        (other, _) => {
            Err(TexError::Invalid(format!("the thumbnail names a kind the editor does not know: {other}")))
        }
    }
}

/// Reads whichever thumbnail the file is.
pub fn read_thm(bytes: &[u8]) -> Result<Thm> {
    Ok(match thm_kind(bytes)? {
        ThmKind::Texture => Thm::Texture(read_texture_thm(bytes)?),
        ThmKind::Object => Thm::Object(read_object_thm(bytes)?),
        ThmKind::Group => Thm::Group(read_group_thm(bytes)?),
        ThmKind::Sound => Thm::Sound(gz_fmt_sound::read_sound_thm(bytes).map_err(|e| match e {
            SoundError::Chunk(c) => TexError::Chunk(c),
            other => TexError::Invalid(other.to_string()),
        })?),
    })
}

/// The file's picture into the model's order, or back: the rows turn over and red swaps with
/// blue, so the same pass serves both ways.
fn turn_pixels(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < THUMB_BYTES {
        return Err(TexError::Truncated { need: THUMB_BYTES - raw.len() });
    }
    let mut out = vec![0u8; THUMB_BYTES];
    for (dst, src) in out
        .as_chunks_mut::<THUMB_ROW>()
        .0
        .iter_mut()
        .zip(raw[..THUMB_BYTES].as_chunks::<THUMB_ROW>().0.iter().rev())
    {
        for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }
    Ok(out)
}

fn read_pixels(file: &Reader<'_>) -> Result<Option<Vec<u8>>> {
    match file.open_chunk(THM_CHUNK_DATA)? {
        Some(chunk) => Ok(Some(turn_pixels(chunk.bytes())?)),
        None => Ok(None),
    }
}

fn write_pixels(w: &mut Writer, pixels: &[u8]) -> Result<()> {
    if pixels.len() != THUMB_BYTES {
        return Err(TexError::BadPixels {
            width: THUMB_SIDE,
            height: THUMB_SIDE,
            need: THUMB_BYTES,
            got: pixels.len(),
        });
    }
    w.w_chunk_compressed(THM_CHUNK_DATA, &turn_pixels(pixels)?);
    Ok(())
}

fn expect(version: u16, want: u16, ty: u32, want_ty: u32) -> Result<()> {
    if version != want {
        return Err(TexError::ThmVersion(version));
    }
    if ty != want_ty {
        return Err(TexError::Invalid(format!("the thumbnail is of kind {ty}, expected {want_ty}")));
    }
    Ok(())
}

/// Reads a texture description.
pub fn read_texture_thm(bytes: &[u8]) -> Result<TextureThm> {
    let file = Reader::new(bytes);
    expect(version_of(&file)?, THM_TEXTURE_VERSION, type_of(&file)?, TYPE_TEXTURE)?;
    Ok(TextureThm { pixels: read_pixels(&file)?, params: TextureParams::read(&file)? })
}

/// Writes a texture description. Without a picture the bytes are
/// what today's editor writes.
pub fn write_texture_thm(thm: &TextureThm) -> Result<Vec<u8>> {
    let mut w = Writer::with_capacity(256);
    w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_TEXTURE_VERSION));
    if let Some(pixels) = &thm.pixels {
        write_pixels(&mut w, pixels)?;
    }
    w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_TEXTURE));
    thm.params.write(&mut w);
    Ok(w.into_bytes())
}

/// Reads a library object's thumbnail.
pub fn read_object_thm(bytes: &[u8]) -> Result<ObjectThm> {
    let file = Reader::new(bytes);
    expect(version_of(&file)?, THM_OBJECT_VERSION, type_of(&file)?, TYPE_OBJECT)?;
    let pixels = read_pixels(&file)?.ok_or(gz_chunk::ChunkError::Missing(THM_CHUNK_DATA))?;
    let chunk = file.need_chunk(THM_CHUNK_OBJECTPARAM)?;
    let mut r = chunk.reader();
    Ok(ObjectThm { pixels, face_count: r.r_u32()?, vertex_count: r.r_u32()? })
}

/// Writes a library object's thumbnail.
pub fn write_object_thm(thm: &ObjectThm) -> Result<Vec<u8>> {
    let mut w = Writer::with_capacity(4096);
    w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_OBJECT_VERSION));
    write_pixels(&mut w, &thm.pixels)?;
    w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_OBJECT));
    w.chunk(THM_CHUNK_OBJECTPARAM, |w| {
        w.w_u32(thm.face_count);
        w.w_u32(thm.vertex_count);
    });
    Ok(w.into_bytes())
}

/// Reads a group's thumbnail.
pub fn read_group_thm(bytes: &[u8]) -> Result<GroupThm> {
    let file = Reader::new(bytes);
    expect(version_of(&file)?, THM_GROUP_VERSION, type_of(&file)?, TYPE_OBJECT)?;
    let pixels = read_pixels(&file)?.ok_or(gz_chunk::ChunkError::Missing(THM_CHUNK_DATA))?;
    let chunk = file.need_chunk(THM_CHUNK_GROUPPARAM)?;
    let mut r = chunk.reader();
    let count = r.r_u32()? as usize;
    // every name takes at least its terminator
    if count > r.elapsed() {
        return Err(TexError::Truncated { need: count - r.elapsed() });
    }
    let mut objects = Vec::with_capacity(count);
    for _ in 0..count {
        objects.push(r.r_stringz()?);
    }
    Ok(GroupThm { pixels, objects })
}

/// Writes a group's thumbnail.
pub fn write_group_thm(thm: &GroupThm) -> Result<Vec<u8>> {
    let mut w = Writer::with_capacity(4096);
    w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_GROUP_VERSION));
    write_pixels(&mut w, &thm.pixels)?;
    w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_OBJECT));
    w.chunk(THM_CHUNK_GROUPPARAM, |w| {
        w.w_u32(thm.objects.len() as u32);
        for name in &thm.objects {
            w.w_stringz(name);
        }
    });
    Ok(w.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{TextureFormat, TextureType};

    fn picture() -> Vec<u8> {
        let mut px = vec![0u8; THUMB_BYTES];
        for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, (i / 128) as u8, 7, 255]);
        }
        px
    }

    #[test]
    fn a_texture_thm_without_a_picture_is_what_the_editor_writes() {
        let thm = TextureThm {
            pixels: None,
            params: TextureParams {
                fmt: TextureFormat::DXT5,
                width: 512,
                height: 512,
                ..TextureParams::default()
            },
        };
        let bytes = write_texture_thm(&thm).unwrap();
        // version chunk, then the type chunk, then the parameters
        assert_eq!(&bytes[..10], &[0x10, 0x08, 0, 0, 2, 0, 0, 0, 0x12, 0x00]);
        assert_eq!(&bytes[10..22], &[0x13, 0x08, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&bytes[22..26], &[0x12, 0x08, 0, 0]);
        assert_eq!(thm_kind(&bytes).unwrap(), ThmKind::Texture);
        assert_eq!(read_texture_thm(&bytes).unwrap(), thm);
        assert_eq!(read_thm(&bytes).unwrap(), Thm::Texture(thm));
    }

    #[test]
    fn a_picture_survives_and_lies_bottom_up_in_the_file() {
        let thm = TextureThm { pixels: Some(picture()), params: TextureParams::default() };
        let bytes = write_texture_thm(&thm).unwrap();
        let back = read_texture_thm(&bytes).unwrap();
        assert_eq!(back, thm);
        assert_eq!(back.params.ty, TextureType::IMAGE);

        let raw = Reader::new(&bytes).need_chunk(THM_CHUNK_DATA).unwrap().into_owned();
        let top_left = &picture()[..4];
        let in_file = &raw[THUMB_BYTES - THUMB_ROW..][..4];
        assert_eq!(in_file, &[top_left[2], top_left[1], top_left[0], top_left[3]]);
    }

    #[test]
    fn objects_and_groups_round_trip() {
        let obj = ObjectThm { pixels: picture(), face_count: 1200, vertex_count: 800 };
        let bytes = write_object_thm(&obj).unwrap();
        assert_eq!(thm_kind(&bytes).unwrap(), ThmKind::Object);
        assert_eq!(read_object_thm(&bytes).unwrap(), obj);

        let grp = GroupThm { pixels: picture(), objects: vec!["props\\box".into(), "ящик".into()] };
        let bytes = write_group_thm(&grp).unwrap();
        assert_eq!(thm_kind(&bytes).unwrap(), ThmKind::Group);
        assert_eq!(read_group_thm(&bytes).unwrap(), grp);
        assert!(matches!(read_thm(&bytes).unwrap(), Thm::Group(_)));
    }

    #[test]
    fn a_sound_thm_is_read_through_the_same_door() {
        let snd = SoundThm {
            quality: 0.0,
            min_dist: 1.0,
            max_dist: 50.0,
            max_ai_dist: 40.0,
            base_volume: 0.8,
            game_type: 3,
        };
        let bytes = gz_fmt_sound::write_sound_thm(&snd);
        assert_eq!(thm_kind(&bytes).unwrap(), ThmKind::Sound);
        assert_eq!(read_thm(&bytes).unwrap(), Thm::Sound(snd));
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(thm_kind(&[]).is_err());
        assert!(read_thm(&[1, 2, 3]).is_err());

        // a version the editor does not read
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(0x0011));
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_TEXTURE));
        assert_eq!(read_texture_thm(&w.into_bytes()), Err(TexError::ThmVersion(0x0011)));

        // a group that claims more names than it has bytes
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_GROUP_VERSION));
        w.w_chunk_compressed(THM_CHUNK_DATA, &vec![0u8; THUMB_BYTES]);
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_OBJECT));
        w.chunk(THM_CHUNK_GROUPPARAM, |w| w.w_u32(u32::MAX));
        assert!(matches!(read_group_thm(&w.into_bytes()), Err(TexError::Truncated { .. })));

        // a picture that is too short
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_OBJECT_VERSION));
        w.w_chunk(THM_CHUNK_DATA, &[0u8; 100]);
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_OBJECT));
        assert!(matches!(read_object_thm(&w.into_bytes()), Err(TexError::Truncated { .. })));

        let bad = ObjectThm { pixels: vec![0; 10], face_count: 0, vertex_count: 0 };
        assert!(matches!(write_object_thm(&bad), Err(TexError::BadPixels { .. })));
    }
}
