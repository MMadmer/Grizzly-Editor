//! Decoded sound: for the preview player and the waveform.

use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;

use crate::ogg;
use crate::{Result, SoundError};

/// The most samples (all channels together) [`decode_ogg`] returns: about fifty minutes of
/// 44.1 kHz stereo. A file that decodes to more is refused rather than taking the memory.
pub const MAX_DECODED_SAMPLES: usize = 1 << 28;

/// A sound's shape, read from its headers and its last page.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OggInfo {
    pub channels: u8,
    pub rate: u32,
    /// Sample frames in the stream, 0 when the file does not say.
    pub frames: u64,
}

impl OggInfo {
    /// The length in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames as f64 / f64::from(self.rate) }
    }

    /// Whether the engine takes the sound: it refuses any rate but 44100 Hz.
    #[must_use]
    pub fn game_rate(&self) -> bool {
        self.rate == 44100
    }
}

/// Channels, rate and length, without decoding the audio.
pub fn ogg_info(ogg: &[u8]) -> Result<OggInfo> {
    let pages = ogg::parse_pages(ogg)?;
    let headers = ogg::read_headers(&pages)?;
    let ident = lewton::header::read_header_ident(&headers.ident)
        .map_err(|_| SoundError::NotOgg("the first packet is not a Vorbis identification header"))?;
    // the last page of the stream that closes a packet says where the sound ends
    let frames = pages
        .iter()
        .rev()
        .find(|p| p.serial == headers.serial && p.granule != u64::MAX)
        .map_or(0, |p| p.granule);
    Ok(OggInfo { channels: ident.audio_channels, rate: ident.audio_sample_rate, frames })
}

/// Decoded audio: `i16` samples, channels interleaved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub channels: u8,
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// The lowest and highest sample of one stretch of a sound.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Peak {
    pub min: i16,
    pub max: i16,
}

impl Pcm {
    /// Sample frames: samples per channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        match self.channels {
            0 => 0,
            n => self.samples.len() / usize::from(n),
        }
    }

    /// The length in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames() as f64 / f64::from(self.rate) }
    }

    /// The envelope a waveform is drawn from: the sound cut into `buckets` equal stretches, each
    /// with its lowest and highest sample over all channels. Fewer buckets come back when the
    /// sound has fewer frames; none for an empty sound.
    #[must_use]
    pub fn peaks(&self, buckets: usize) -> Vec<Peak> {
        let channels = usize::from(self.channels).max(1);
        let frames = self.samples.len() / channels;
        let buckets = buckets.min(frames);
        let mut out = Vec::with_capacity(buckets);
        for b in 0..buckets {
            // widened: a long sound times a bucket index overflows 32 bits
            let from = (b as u128 * frames as u128 / buckets as u128) as usize;
            let to =
                (((b + 1) as u128 * frames as u128 / buckets as u128) as usize).max(from + 1).min(frames);
            let stretch = self.samples.get(from * channels..to * channels).unwrap_or(&[]);
            let mut peak = Peak { min: i16::MAX, max: i16::MIN };
            for &s in stretch {
                peak.min = peak.min.min(s);
                peak.max = peak.max.max(s);
            }
            out.push(if stretch.is_empty() { Peak::default() } else { peak });
        }
        out
    }
}

/// Decodes a whole `.ogg`. The game's comment block does not disturb the decoder.
pub fn decode_ogg(ogg: &[u8]) -> Result<Pcm> {
    let mut reader = OggStreamReader::new(Cursor::new(ogg)).map_err(|e| SoundError::Decode(e.to_string()))?;
    let channels = reader.ident_hdr.audio_channels;
    let rate = reader.ident_hdr.audio_sample_rate;
    let mut samples: Vec<i16> = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| SoundError::Decode(e.to_string()))? {
        if samples.len() + packet.len() > MAX_DECODED_SAMPLES {
            return Err(SoundError::TooLong);
        }
        samples.extend_from_slice(&packet);
    }
    Ok(Pcm { channels, rate, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_cover_every_frame_once() {
        let pcm = Pcm { channels: 2, rate: 44100, samples: vec![1, -1, 5, -7, 3, 2, -9, 4, 0, 0] };
        assert_eq!(pcm.frames(), 5);
        let peaks = pcm.peaks(2);
        // frames 0..2 and 2..5
        assert_eq!(peaks, vec![Peak { min: -7, max: 5 }, Peak { min: -9, max: 4 }]);
        // more buckets than frames: one per frame
        assert_eq!(pcm.peaks(100).len(), 5);
        assert_eq!(pcm.peaks(0), vec![]);
        assert!((pcm.seconds() - 5.0 / 44100.0).abs() < 1e-12);
    }

    #[test]
    fn an_empty_or_channelless_sound_has_no_peaks() {
        let pcm = Pcm { channels: 0, rate: 0, samples: vec![] };
        assert_eq!(pcm.frames(), 0);
        assert_eq!(pcm.seconds(), 0.0);
        assert!(pcm.peaks(8).is_empty());
    }

    #[test]
    fn garbage_does_not_decode() {
        assert!(matches!(decode_ogg(&[]), Err(SoundError::Decode(_))));
        assert!(matches!(decode_ogg(b"OggS but not really"), Err(SoundError::Decode(_))));
        assert!(ogg_info(b"nothing").is_err());
        let fake = crate::params::tests::fake_ogg(b"v", &[], 40);
        // the headers are Vorbis in name only
        assert!(ogg_info(&fake).is_err());
        assert!(decode_ogg(&fake).is_err());
    }
}
