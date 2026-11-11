//! The sound thumbnail (`ESoundThumbnail`): the parameters the editor keeps beside a `.wav`.

use gz_chunk::{Reader, Writer};

use crate::{Result, SoundError};

const THM_CHUNK_VERSION: u32 = 0x0810;
const THM_CHUNK_TYPE: u32 = 0x0813;
const THM_CHUNK_SOUNDPARAM: u32 = 0x1000;
const THM_CHUNK_SOUNDPARAM2: u32 = 0x1001;
const THM_CHUNK_SOUND_AI_DIST: u32 = 0x1002;

const THM_SOUND_VERSION: u16 = 0x0014;
const TYPE_SOUND: u32 = 2;

/// A sound's description.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SoundThm {
    /// The Vorbis quality the sound is encoded with.
    pub quality: f32,
    pub min_dist: f32,
    pub max_dist: f32,
    /// How far NPCs hear the sound; at most `max_dist`.
    pub max_ai_dist: f32,
    pub base_volume: f32,
    /// The engine's sound type (`ESoundTypes`).
    pub game_type: u32,
}

impl Default for SoundThm {
    fn default() -> Self {
        Self {
            quality: 0.0,
            min_dist: 1.0,
            max_dist: 300.0,
            max_ai_dist: 300.0,
            base_volume: 1.0,
            game_type: 0,
        }
    }
}

/// Reads a sound thumbnail (`ESoundThumbnail::Load`).
pub fn read_sound_thm(bytes: &[u8]) -> Result<SoundThm> {
    let file = Reader::new(bytes);
    let version = file.need_chunk(THM_CHUNK_VERSION)?.reader().r_u16()?;
    if version != THM_SOUND_VERSION {
        return Err(SoundError::ThmVersion(version));
    }
    let ty = file.need_chunk(THM_CHUNK_TYPE)?.reader().r_u32()?;
    if ty != TYPE_SOUND {
        return Err(SoundError::ThmKind(ty));
    }

    let mut thm = SoundThm::default();
    let main = file.need_chunk(THM_CHUNK_SOUNDPARAM)?;
    let mut r = main.reader();
    thm.quality = r.r_f32()?;
    thm.min_dist = r.r_f32()?;
    thm.max_dist = r.r_f32()?;
    thm.game_type = r.r_u32()?;

    if let Some(c) = file.open_chunk(THM_CHUNK_SOUNDPARAM2)? {
        thm.base_volume = c.reader().r_f32()?;
    }
    thm.max_ai_dist = match file.open_chunk(THM_CHUNK_SOUND_AI_DIST)? {
        Some(c) => c.reader().r_f32()?,
        None => thm.max_dist,
    };
    Ok(thm)
}

/// Writes a sound thumbnail (`ESoundThumbnail::Save`).
#[must_use]
pub fn write_sound_thm(thm: &SoundThm) -> Vec<u8> {
    let mut w = Writer::with_capacity(96);
    w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_SOUND_VERSION));
    w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_SOUND));
    w.chunk(THM_CHUNK_SOUNDPARAM, |w| {
        w.w_f32(thm.quality);
        w.w_f32(thm.min_dist);
        w.w_f32(thm.max_dist);
        w.w_u32(thm.game_type);
    });
    w.chunk(THM_CHUNK_SOUNDPARAM2, |w| w.w_f32(thm.base_volume));
    w.chunk(THM_CHUNK_SOUND_AI_DIST, |w| w.w_f32(thm.max_ai_dist));
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_has_the_editors_layout() {
        let thm = SoundThm {
            quality: 0.3,
            min_dist: 2.0,
            max_dist: 80.0,
            max_ai_dist: 60.0,
            base_volume: 1.5,
            game_type: 0x0800_0000,
        };
        let bytes = write_sound_thm(&thm);
        assert_eq!(bytes.len(), 10 + 12 + 24 + 12 + 12);
        assert_eq!(&bytes[..10], &[0x10, 0x08, 0, 0, 2, 0, 0, 0, 0x14, 0x00]);
        assert_eq!(read_sound_thm(&bytes).unwrap(), thm);
    }

    #[test]
    fn an_old_file_takes_its_ai_distance_from_the_max_distance() {
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_SOUND_VERSION));
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_SOUND));
        w.chunk(THM_CHUNK_SOUNDPARAM, |w| {
            w.w_f32(0.0);
            w.w_f32(1.0);
            w.w_f32(42.0);
            w.w_u32(7);
        });
        let thm = read_sound_thm(&w.into_bytes()).unwrap();
        assert_eq!(thm.max_ai_dist, 42.0);
        assert_eq!(thm.base_volume, 1.0);
        assert_eq!(thm.game_type, 7);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(read_sound_thm(&[]).is_err());
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(0x0012));
        assert_eq!(read_sound_thm(&w.into_bytes()), Err(SoundError::ThmVersion(0x0012)));

        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_SOUND_VERSION));
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(1));
        assert_eq!(read_sound_thm(&w.into_bytes()), Err(SoundError::ThmKind(1)));

        let mut w = Writer::new();
        w.chunk(THM_CHUNK_VERSION, |w| w.w_u16(THM_SOUND_VERSION));
        w.chunk(THM_CHUNK_TYPE, |w| w.w_u32(TYPE_SOUND));
        w.chunk(THM_CHUNK_SOUNDPARAM, |w| w.w_f32(0.0));
        assert!(matches!(read_sound_thm(&w.into_bytes()), Err(SoundError::Chunk(_))));
    }
}
