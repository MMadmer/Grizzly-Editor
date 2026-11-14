//! `senvironment.xr`: the sound environments (EAX presets).

use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, numbered, w_str};

/// `sdef_env_version`.
pub const SENV_VERSION: u32 = 4;

/// A sound environment.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundEnvironment {
    /// The version it was read with; it is written as version 4.
    pub version: u32,
    pub name: SmolStr,
    pub room: f32,
    pub room_hf: f32,
    pub room_rolloff_factor: f32,
    pub decay_time: f32,
    pub decay_hf_ratio: f32,
    pub reflections: f32,
    pub reflections_delay: f32,
    pub reverb: f32,
    pub reverb_delay: f32,
    pub environment_size: f32,
    pub environment_diffusion: f32,
    pub air_absorption_hf: f32,
    /// Version 4 and later; 0 before.
    pub environment: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundEnvironments {
    pub environments: Vec<SoundEnvironment>,
    pub encoding: Encoding,
}

impl SoundEnvironments {
    /// An environment older than version 3 is skipped.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let mut text = Text::default();
        let mut environments = Vec::new();
        for c in numbered(&r)? {
            let mut e = c.reader();
            let version = e.r_u32()?;
            if version < 3 {
                continue;
            }
            let name = text.r_str(&mut e)?;
            let mut f = [0.0f32; 12];
            for v in &mut f {
                *v = e.r_f32()?;
            }
            let environment = if version > 3 { e.r_u32()? } else { 0 };
            environments.push(SoundEnvironment {
                version,
                name,
                room: f[0],
                room_hf: f[1],
                room_rolloff_factor: f[2],
                decay_time: f[3],
                decay_hf_ratio: f[4],
                reflections: f[5],
                reflections_delay: f[6],
                reverb: f[7],
                reverb_delay: f[8],
                environment_size: f[9],
                environment_diffusion: f[10],
                air_absorption_hf: f[11],
                environment,
            });
        }
        Ok(Self { environments, encoding: text.encoding() })
    }

    /// Saves the sound environment library.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for (i, e) in self.environments.iter().enumerate() {
            w.chunk(i as u32, |w| {
                w.w_u32(SENV_VERSION);
                w_str(w, &e.name, self.encoding);
                for v in [
                    e.room,
                    e.room_hf,
                    e.room_rolloff_factor,
                    e.decay_time,
                    e.decay_hf_ratio,
                    e.reflections,
                    e.reflections_delay,
                    e.reverb,
                    e.reverb_delay,
                    e.environment_size,
                    e.environment_diffusion,
                    e.air_absorption_hf,
                ] {
                    w.w_f32(v);
                }
                w.w_u32(e.environment);
            });
        }
        w.into_bytes()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SoundEnvironment> {
        self.environments.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environments_write_read_and_write_the_same_bytes() {
        let e = SoundEnvironment {
            version: 4,
            name: "cave".into(),
            room: -1000.0,
            room_hf: -100.0,
            room_rolloff_factor: 0.0,
            decay_time: 2.9,
            decay_hf_ratio: 1.3,
            reflections: -602.0,
            reflections_delay: 0.015,
            reverb: -302.0,
            reverb_delay: 0.022,
            environment_size: 14.6,
            environment_diffusion: 1.0,
            air_absorption_hf: -5.0,
            environment: 8,
        };
        let lib = SoundEnvironments { environments: vec![e], encoding: Encoding::Utf8 };
        let b = lib.write();
        let back = SoundEnvironments::read(&b).unwrap();
        assert_eq!(back, lib);
        assert_eq!(back.write(), b);
        let mut old = Writer::new();
        old.chunk(0, |w| w.w_u32(2));
        assert!(SoundEnvironments::read(&old.into_bytes()).unwrap().environments.is_empty());
    }
}
