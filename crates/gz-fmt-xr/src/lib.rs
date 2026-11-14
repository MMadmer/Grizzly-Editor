//! The engine's `.xr` libraries.
//!
//! ```text
//! LightAnims::read / write, item.color_at(frame), item.calculate(t)     lanims.xr
//! GameMaterials::read                                                    gamemtl.xr
//! Shaders::read                                                          shaders.xr
//! ShadersXrlc::read / write                                              shaders_xrlc.xr
//! Particles::read  (effects, groups, the action stream decoded)          particles.xr
//! SoundEnvironments::read / write                                        senvironment.xr
//! ```
//!
//! Readers take `&[u8]`; writers return `Vec<u8>` and write what the X-Ray SDK editor writes. Names
//! are read as UTF-8 when they are and as cp1251 otherwise, and written back the way they came.
//! Hostile input is an `Err`.

mod gamemtl;
mod lanims;
mod particles;
mod senv;
mod shaders;

pub use gamemtl::{GameMaterial, GameMaterialPair, GameMaterials};
pub use lanims::{LightAnim, LightAnims};
pub use particles::{
    Action, ActionKind, ActionParams, Domain, EffectFrame, GroupEffect, ParticleEffect, ParticleGroup,
    Particles, effect_flags,
};
pub use senv::{SoundEnvironment, SoundEnvironments};
pub use shaders::{Blender, MatrixDef, ShaderXrlc, Shaders, ShadersXrlc, WaveForm};

use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum XrError {
    #[error(transparent)]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = XrError> = std::result::Result<T, E>;

pub(crate) fn invalid(what: impl Into<String>) -> XrError {
    XrError::Invalid(what.into())
}

/// How a library's strings are stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// What the editor's UTF-8 process writes.
    #[default]
    Utf8,
    /// The game's own files.
    Cp1251,
}

/// Reads strings and remembers whether one was cp1251.
#[derive(Default)]
pub(crate) struct Text {
    pub(crate) cp1251: bool,
}

impl Text {
    pub(crate) fn r_str(&mut self, r: &mut Reader<'_>) -> Result<SmolStr> {
        let raw = r.r_stringz_raw()?;
        Ok(SmolStr::new(match std::str::from_utf8(raw) {
            Ok(s) => s.to_owned(),
            Err(_) => {
                self.cp1251 = true;
                gz_base::text::cp1251_to_utf8(raw).into_owned()
            }
        }))
    }

    pub(crate) fn encoding(&self) -> Encoding {
        if self.cp1251 { Encoding::Cp1251 } else { Encoding::Utf8 }
    }
}

pub(crate) fn w_str(w: &mut Writer, s: &str, enc: Encoding) {
    match enc {
        Encoding::Utf8 => w.w_stringz(s),
        Encoding::Cp1251 => w.w_stringz_cp1251(s),
    }
}

/// Sub-chunks `0, 1, 2, ...` up to the first one that is missing (`open_chunk(count)` loops).
pub(crate) fn numbered<'a>(r: &Reader<'a>) -> Result<Vec<gz_chunk::Chunk<'a>>> {
    let mut out = Vec::new();
    for id in 0u32.. {
        match r.open_chunk(id)? {
            Some(c) => out.push(c),
            None => break,
        }
    }
    Ok(out)
}
