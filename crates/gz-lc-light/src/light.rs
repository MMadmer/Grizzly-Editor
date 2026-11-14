//! The lights of a build: the three sets of the base lighting (static, hemisphere,
//! sun), the selection of a sphere's lights, the conversion from `build.prj`
//! and `build.lights`, which the detail compiler reads.

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use gz_fmt_level::{BuildLight, LightControl};

use crate::math::{EPS_S, distance, normalize_safe};
use crate::{LcError, Result};

/// `LCONTROL_HEMI`, `LCONTROL_SUN`, `LCONTROL_STATIC`.
pub const LCONTROL_HEMI: &str = "$hemi";
pub const LCONTROL_SUN: &str = "$sun";
pub const LCONTROL_STATIC: &str = "$static";

/// `LT_DIRECT`, `LT_POINT`, `LT_SECONDARY`.
pub const LT_DIRECT: u16 = 0;
pub const LT_POINT: u16 = 1;
pub const LT_SECONDARY: u16 = 2;

/// `D3DLIGHT_DIRECTIONAL`.
const D3DLIGHT_DIRECTIONAL: u32 = 3;

/// The size of a light record in `build.lights`.
pub const R_LIGHT_SIZE: usize = 104;

/// A light. X-Ray's compiler also keeps the triangle that last shadowed it here; the compiler keeps
/// that per worker ([`crate::ray::LightCache`]), so a light set is shared, never written.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RLight {
    pub kind: u16,
    /// The radiosity level.
    pub level: u16,
    pub diffuse: Vec3,
    pub position: Vec3,
    pub direction: Vec3,
    pub range: f32,
    pub range2: f32,
    /// Makes the light 0 at its range.
    pub falloff: f32,
    pub attenuation0: f32,
    pub attenuation1: f32,
    pub attenuation2: f32,
    pub energy: f32,
}

impl RLight {
    /// The light of a `Flight`: its colour split into a direction of colour and
    /// an energy, the range stretched by a tenth.
    #[must_use]
    pub fn from_flight(l: &gz_fmt_level::Light) -> Self {
        let kind = if l.kind == D3DLIGHT_DIRECTIONAL { LT_DIRECT } else { LT_POINT };
        let [r, g, b, _] = l.diffuse;
        let mut e = (r + g + b) / 3.0;
        let mut c = Vec3::new(r, g, b);
        if e.abs() > EPS_S {
            c = Vec3::new(c.x / e, c.y / e, c.z / e);
        } else {
            c = Vec3::ZERO;
            e = 0.0;
        }
        let range = l.range * 1.1;
        let range2 = range * range;
        Self {
            kind,
            level: 0,
            diffuse: c,
            position: l.position,
            direction: normalize_safe(l.direction),
            range,
            range2,
            attenuation0: l.attenuation0,
            attenuation1: l.attenuation1,
            attenuation2: l.attenuation2,
            falloff: 1.0 / (range * (l.attenuation0 + l.attenuation1 * range + l.attenuation2 * range2)),
            energy: e,
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_u16(self.kind);
        w.w_u16(self.level);
        w.w_vec3(self.diffuse);
        w.w_vec3(self.position);
        w.w_vec3(self.direction);
        for v in [
            self.range,
            self.range2,
            self.falloff,
            self.attenuation0,
            self.attenuation1,
            self.attenuation2,
            self.energy,
        ] {
            w.w_f32(v);
        }
        // the cached triangle as `R_Light()` leaves it
        w.w_vec3(Vec3::ZERO);
        w.w_vec3(Vec3::new(0.0, 0.0, EPS_S));
        w.w_vec3(Vec3::new(EPS_S, 0.0, 0.0));
    }

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let l = Self {
            kind: r.r_u16()?,
            level: r.r_u16()?,
            diffuse: r.r_vec3()?,
            position: r.r_vec3()?,
            direction: r.r_vec3()?,
            range: r.r_f32()?,
            range2: r.r_f32()?,
            falloff: r.r_f32()?,
            attenuation0: r.r_f32()?,
            attenuation1: r.r_f32()?,
            attenuation2: r.r_f32()?,
            energy: r.r_f32()?,
        };
        r.advance(36)?;
        Ok(l)
    }
}

/// `base_lighting`: the static lights, the hemisphere's and the sun's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightSet {
    pub rgb: Vec<RLight>,
    pub hemi: Vec<RLight>,
    pub sun: Vec<RLight>,
}

fn select_into(dest: &mut Vec<RLight>, src: &[RLight], p: Vec3, r: f32) {
    dest.clear();
    for l in src {
        if l.kind == LT_POINT && distance(p, l.position) > r + l.range {
            continue;
        }
        dest.push(*l);
    }
}

impl LightSet {
    /// `base_lighting::select`: the lights that may reach a sphere (a point light whose range
    /// does not is left out), into `self`.
    pub fn select_from(&mut self, from: &LightSet, p: Vec3, r: f32) {
        select_into(&mut self.rgb, &from.rgb, p, r);
        select_into(&mut self.hemi, &from.hemi, p, r);
        select_into(&mut self.sun, &from.sun, p, r);
    }

    /// The selection as a new set.
    #[must_use]
    pub fn select(&self, p: Vec3, r: f32) -> LightSet {
        let mut out = LightSet::default();
        out.select_from(self, p, r);
        out
    }

    /// The lights of `build.prj`: every static light goes to its controller's layer, and the
    /// layers named `$hemi`, `$sun` and `$static` become the three sets. A build without a
    /// hemisphere or a sun is refused.
    pub fn from_project(controls: &[LightControl], statics: &[BuildLight]) -> Result<Self> {
        let mut layers: Vec<Vec<RLight>> = vec![Vec::new(); controls.len()];
        for l in statics {
            let Some(layer) = layers.get_mut(l.controller as usize) else {
                return Err(LcError::Invalid(format!(
                    "A light names the controller {}, the build has {}.",
                    l.controller,
                    controls.len()
                )));
            };
            layer.push(RLight::from_flight(&l.light));
        }
        let mut set = LightSet::default();
        for (control, layer) in controls.iter().zip(layers) {
            let name = control.name.as_str();
            if name.eq_ignore_ascii_case(LCONTROL_HEMI) {
                set.hemi.clone_from(&layer);
            }
            if name.eq_ignore_ascii_case(LCONTROL_SUN) {
                set.sun.clone_from(&layer);
            }
            if name.eq_ignore_ascii_case(LCONTROL_STATIC) {
                set.rgb = layer;
            }
        }
        if set.hemi.is_empty() {
            return Err(LcError::Invalid("The build has no hemisphere lights.".into()));
        }
        if set.sun.is_empty() {
            return Err(LcError::Invalid("The build has no sun.".into()));
        }
        Ok(set)
    }

    /// `build.lights`: chunks 0, 1 and 2 hold the static, the
    /// hemisphere and the sun lights.
    #[must_use]
    pub fn write_build_lights(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for (id, set) in [(0, &self.rgb), (1, &self.hemi), (2, &self.sun)] {
            w.chunk(id, |w| set.iter().for_each(|l| l.write(w)));
        }
        w.into_bytes()
    }

    /// Reads `build.lights`.
    pub fn read_build_lights(bytes: &[u8]) -> Result<Self> {
        let file = Reader::new(bytes);
        let mut out = LightSet::default();
        for (id, set) in [(0, &mut out.rgb), (1, &mut out.hemi), (2, &mut out.sun)] {
            let chunk = file.need_chunk(id)?;
            let mut r = chunk.reader();
            while r.elapsed() >= R_LIGHT_SIZE {
                set.push(RLight::read(&mut r)?);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_light_is_split_into_colour_and_energy() {
        let f = gz_fmt_level::Light {
            kind: 1,
            diffuse: [3.0, 1.5, 1.5, 1.0],
            position: Vec3::new(1.0, 2.0, 3.0),
            range: 10.0,
            attenuation0: 1.0,
            ..Default::default()
        };
        let l = RLight::from_flight(&f);
        assert_eq!(l.kind, LT_POINT);
        assert_eq!(l.energy, 2.0);
        assert_eq!(l.diffuse, Vec3::new(1.5, 0.75, 0.75));
        assert_eq!(l.range, 11.0);
        assert_eq!(l.falloff, 1.0 / 11.0);
    }

    #[test]
    fn selection_drops_point_lights_out_of_reach_and_the_file_round_trips() {
        let near =
            RLight { kind: LT_POINT, position: Vec3::new(5.0, 0.0, 0.0), range: 2.0, ..Default::default() };
        let far = RLight { position: Vec3::new(50.0, 0.0, 0.0), ..near };
        let sun = RLight { kind: LT_DIRECT, direction: Vec3::NEG_Y, energy: 1.0, ..Default::default() };
        let all = LightSet { rgb: vec![near, far], hemi: vec![sun], sun: vec![sun] };
        let picked = all.select(Vec3::ZERO, 4.0);
        assert_eq!(picked.rgb, vec![near]);
        assert_eq!(picked.sun.len(), 1);
        let bytes = all.write_build_lights();
        assert_eq!(bytes.len(), 3 * 8 + 4 * R_LIGHT_SIZE);
        assert_eq!(LightSet::read_build_lights(&bytes).unwrap(), all);
    }
}
