//! Making a game sound from a `.wav` (`CSoundManager::MakeGameSound`): the samples encoded as
//! Ogg Vorbis at the description's quality, the game's parameters as the stream's first comment.
//!
//! The C++ editor ran `oggenc`'s code with VBR at the description's quality, stream serial 0 and
//! one binary comment; this is the same through `vorbis_rs` (libvorbis with the aoTuV tunings,
//! so the bytes differ from the C++ file while the stream and its comment read the same). The
//! encoder takes text comments only, so the block is put into the comment header afterwards by
//! [`write_ogg_params`], which lays the header pages out again.

use std::num::{NonZeroU8, NonZeroU32};

use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

use crate::params::{OggParams, write_ogg_params};
use crate::{Result, SoundError};

/// The most sample frames a `.wav` may hold to be encoded: about an hour and a half at
/// 44.1 kHz. A larger one is refused rather than taking the memory.
pub const MAX_WAV_FRAMES: usize = 1 << 28;

/// How many frames go to the encoder at once (libvorbis: "1024 is a reasonable choice").
const BLOCK: usize = 4096;

/// A `.wav` as the encoder takes it: planar `f32` in -1..1.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub rate: u32,
    /// One vector of samples per channel, all the same length.
    pub channels: Vec<Vec<f32>>,
}

impl Wav {
    /// Sample frames: samples per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

fn bad(what: &'static str) -> SoundError {
    SoundError::Wav(what)
}

/// Reads a RIFF WAVE file: integer PCM of 8, 16, 24 or 32 bits, or 32-bit float, any number of
/// channels up to 255, also in a `WAVE_FORMAT_EXTENSIBLE` header.
pub fn read_wav(bytes: &[u8]) -> Result<Wav> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(bad("it is not a RIFF WAVE file"));
    }
    let mut at = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while at + 8 <= bytes.len() {
        let id = bytes.get(at..at + 4).unwrap_or(&[]);
        let size = le_u32(bytes, at + 4).ok_or(bad("a chunk header is cut short"))? as usize;
        let body_at = at + 8;
        // a data chunk that claims more than the file holds is read to the end of the file
        let body = bytes.get(body_at..body_at.saturating_add(size).min(bytes.len())).unwrap_or(&[]);
        match id {
            b"fmt " => {
                let tag = le_u16(body, 0).ok_or(bad("the format chunk is cut short"))?;
                let channels = le_u16(body, 2).ok_or(bad("the format chunk is cut short"))?;
                let rate = le_u32(body, 4).ok_or(bad("the format chunk is cut short"))?;
                let bits = le_u16(body, 14).ok_or(bad("the format chunk is cut short"))?;
                // the extensible header names the real format in the first two bytes of its GUID
                let tag = if tag == WAVE_FORMAT_EXTENSIBLE {
                    le_u16(body, 24).ok_or(bad("the extensible format chunk is cut short"))?
                } else {
                    tag
                };
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // chunks are padded to an even size
        at = body_at.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or(bad("it has no format chunk"))?;
    let data = data.ok_or(bad("it has no data chunk"))?;
    if channels == 0 || channels > 255 {
        return Err(bad("its channel count is not 1..255"));
    }
    if rate == 0 {
        return Err(bad("its sample rate is 0"));
    }
    let width = match (tag, bits) {
        (WAVE_FORMAT_PCM, 8) => 1,
        (WAVE_FORMAT_PCM, 16) => 2,
        (WAVE_FORMAT_PCM, 24) => 3,
        (WAVE_FORMAT_PCM, 32) | (WAVE_FORMAT_IEEE_FLOAT, 32) => 4,
        _ => return Err(bad("its sample format is not 8, 16, 24 or 32-bit PCM or 32-bit float")),
    };
    let n = usize::from(channels);
    let frame = width * n;
    let frames = data.len() / frame;
    if frames > MAX_WAV_FRAMES {
        return Err(SoundError::TooLong);
    }
    let mut out: Vec<Vec<f32>> = (0..n).map(|_| Vec::with_capacity(frames)).collect();
    for f in data.chunks_exact(frame) {
        for (ch, s) in out.iter_mut().zip(f.chunks_exact(width)) {
            let v = match (tag, width) {
                (_, 1) => (f32::from(s[0]) - 128.0) / 128.0,
                (_, 2) => f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0,
                (_, 3) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                (WAVE_FORMAT_IEEE_FLOAT, _) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            };
            ch.push(v);
        }
    }
    Ok(Wav { rate, channels: out })
}

/// Encodes `wav` as a game sound: Ogg Vorbis, VBR at `quality` (the description's 0..1), stream
/// serial 0, `params` as the first comment.
pub fn encode_game_sound(wav: &Wav, quality: f32, params: &OggParams) -> Result<Vec<u8>> {
    let rate = NonZeroU32::new(wav.rate).ok_or(bad("its sample rate is 0"))?;
    let channels = u8::try_from(wav.channels.len())
        .ok()
        .and_then(NonZeroU8::new)
        .ok_or(bad("its channel count is not 1..255"))?;
    let encode_err = |e: vorbis_rs::VorbisError| SoundError::Encode(e.to_string());
    let mut builder = VorbisEncoderBuilder::new_with_serial(rate, channels, Vec::new(), 0);
    builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: quality.clamp(-0.1, 1.0),
    });
    let mut encoder = builder.build().map_err(encode_err)?;
    let frames = wav.frames();
    let mut start = 0;
    while start < frames {
        let end = (start + BLOCK).min(frames);
        let block: Vec<&[f32]> = wav.channels.iter().map(|c| c.get(start..end).unwrap_or(&[])).collect();
        encoder.encode_audio_block(&block).map_err(encode_err)?;
        start = end;
    }
    let ogg = encoder.finish().map_err(encode_err)?;
    write_ogg_params(&ogg, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav16(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn a_wav_reads_planar() {
        let w = read_wav(&wav16(44100, 2, &[0, 16384, -32768, 0])).unwrap();
        assert_eq!(w.rate, 44100);
        assert_eq!(w.channels, vec![vec![0.0, -1.0], vec![0.5, 0.0]]);
        assert!(read_wav(b"RIFF\0\0\0\0WAVE").is_err());
        assert!(read_wav(b"nope").is_err());
    }

    #[test]
    fn a_game_sound_carries_its_parameters() {
        let samples: Vec<i16> = (0..44100).map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16).collect();
        let wav = read_wav(&wav16(44100, 1, &samples)).unwrap();
        let params =
            OggParams { min_dist: 1.0, max_dist: 30.0, base_volume: 1.0, game_type: 0, max_ai_dist: 30.0 };
        let ogg = encode_game_sound(&wav, 0.0, &params).unwrap();
        assert_eq!(crate::read_ogg_params(&ogg).unwrap(), Some(params));
        let info = crate::ogg_info(&ogg).unwrap();
        assert_eq!((info.channels, info.rate, info.frames), (1, 44100, 44100));
    }
}
