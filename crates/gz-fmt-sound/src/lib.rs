//! What the editor reads and writes of a game sound.
//!
//! A game sound is an Ogg Vorbis file whose first comment is not text: it is the block of
//! parameters the engine plays the sound with. The editor keeps the same parameters, and the
//! encoding quality, in a `.thm` beside the source `.wav`.
//!
//! Every function takes bytes and returns values; hostile input is an `Err`.
//!
//! - [`OggParams`] - the engine's parameters: distances, base volume, game type.
//!   [`OggParams::from_comment`] / [`OggParams::to_comment`] are the comment block itself.
//! - [`read_ogg_params`] - the parameters of an `.ogg`, `None` when it carries no comment.
//! - [`write_ogg_params`] - the same `.ogg` with other parameters: the comment packet is
//!   rewritten and its pages are laid out again, the audio is not touched.
//! - [`ogg_info`] - channels, rate and length without decoding.
//! - [`decode_ogg`] - the sound as interleaved `i16`: [`Pcm`]. [`Pcm::peaks`] is the envelope a
//!   waveform is drawn from.
//! - [`SoundThm`], [`read_sound_thm`], [`write_sound_thm`] - the sound's thumbnail file.
//! - [`read_wav`], [`encode_game_sound`] - a game sound made from its `.wav`: Vorbis at the
//!   description's quality, the parameters as its first comment.

mod encode;
mod ogg;
mod params;
mod pcm;
mod thm;

pub use encode::{MAX_WAV_FRAMES, Wav, encode_game_sound, read_wav};
pub use params::{OGG_COMMENT_VERSION, OggParams, read_ogg_params, write_ogg_params};
pub use pcm::{MAX_DECODED_SAMPLES, OggInfo, Pcm, Peak, decode_ogg, ogg_info};
pub use thm::{SoundThm, read_sound_thm, write_sound_thm};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SoundError {
    #[error("{0}")]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("the thumbnail has version {0:#06x}, which this editor does not read")]
    ThmVersion(u16),
    #[error("the thumbnail is not a sound's (kind {0})")]
    ThmKind(u32),
    #[error("this is not an Ogg Vorbis file: {0}")]
    NotOgg(&'static str),
    #[error("the Ogg file is cut short or damaged: {0}")]
    Damaged(&'static str),
    #[error("the sound's parameters have version {0}, which the game does not read")]
    CommentVersion(u32),
    #[error("the Ogg file is laid out in a way the editor does not rewrite: {0}")]
    Layout(&'static str),
    #[error("the sound could not be decoded: {0}")]
    Decode(String),
    #[error("the sound is longer than the editor decodes at once")]
    TooLong,
    #[error("the .wav cannot be read: {0}")]
    Wav(&'static str),
    #[error("the sound could not be encoded: {0}")]
    Encode(String),
}

pub type Result<T, E = SoundError> = std::result::Result<T, E>;
