//! `gamemtl.xr`: the game materials and the effects of every pair of them.
//! `XrGameMaterialLibraryInterface.cpp` (`SGameMtl::Load`, `SGameMtlPair::Load`), `GameMtlLib.cpp`.

use gz_chunk::Reader;
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, invalid};

const CHUNK_VERSION: u32 = 0x1000;
const CHUNK_AUTOINC: u32 = 0x1001;
const CHUNK_MTLS: u32 = 0x1002;
const CHUNK_MTLS_PAIR: u32 = 0x1003;
const MTL_MAIN: u32 = 0x1000;
const MTL_FLAGS: u32 = 0x1001;
const MTL_PHYSICS: u32 = 0x1002;
const MTL_FACTORS: u32 = 0x1003;
const MTL_FLOTATION: u32 = 0x1004;
const MTL_DESC: u32 = 0x1005;
const MTL_INJURIOUS: u32 = 0x1006;
const MTL_DENSITY: u32 = 0x1007;
const MTL_FACTORS_MP: u32 = 0x1008;
const PAIR_PAIR: u32 = 0x1000;
const PAIR_BREAKING: u32 = 0x1002;
const PAIR_STEP: u32 = 0x1003;
const PAIR_COLLIDE: u32 = 0x1005;
/// `GAMEMTL_CURRENT_VERSION`.
pub const GAMEMTL_VERSION: u16 = 1;

/// `SGameMtl`.
#[derive(Clone, Debug, PartialEq)]
pub struct GameMaterial {
    pub id: u32,
    pub name: SmolStr,
    pub desc: SmolStr,
    pub flags: u32,
    pub friction: f32,
    pub damping: f32,
    pub spring: f32,
    pub bounce_start_velocity: f32,
    pub bouncing: f32,
    pub shoot_factor: f32,
    pub bounce_damage_factor: f32,
    pub vis_transparency_factor: f32,
    pub snd_occlusion_factor: f32,
    /// `fShootFactorMP`: the shoot factor when the file has none of its own.
    pub shoot_factor_mp: f32,
    pub flotation_factor: Option<f32>,
    pub injurious_speed: Option<f32>,
    pub density_factor: Option<f32>,
}

impl GameMaterial {
    /// `SGameMtl::flBreakable`.
    pub const BREAKABLE: u32 = 1 << 0;
    /// `SGameMtl::flClimable`.
    pub const CLIMABLE: u32 = 1 << 5;
    /// `SGameMtl::flDynamic` (`XrGameMaterialLibraryInterface.h`: bit 8).
    pub const DYNAMIC: u32 = 1 << 8;

    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        self.flags & Self::DYNAMIC != 0
    }
}

/// `SGameMtlPair`: lists are the comma lists as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct GameMaterialPair {
    pub mtl0: u32,
    pub mtl1: u32,
    pub id: u32,
    pub id_parent: u32,
    pub own_props: u32,
    pub breaking_sounds: SmolStr,
    pub step_sounds: SmolStr,
    pub collide_sounds: SmolStr,
    pub collide_particles: SmolStr,
    pub collide_marks: SmolStr,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameMaterials {
    pub material_index: u32,
    pub material_pair_index: u32,
    pub materials: Vec<GameMaterial>,
    pub pairs: Vec<GameMaterialPair>,
    pub encoding: Encoding,
}

fn need<'a>(r: &Reader<'a>, id: u32, what: &str) -> Result<Reader<'a>> {
    r.sub(id)?.ok_or_else(|| invalid(format!("a game material has no {what}")))
}

impl GameMaterials {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = need(&r, CHUNK_VERSION, "version")?.r_u16()?;
        if version != GAMEMTL_VERSION {
            return Err(invalid(format!(
                "the game materials have version {version}; the game reads version 1"
            )));
        }
        let mut a = need(&r, CHUNK_AUTOINC, "counters")?;
        let mut out = Self { material_index: a.r_u32()?, material_pair_index: a.r_u32()?, ..Self::default() };
        let mut text = Text::default();
        if let Some(list) = r.sub(CHUNK_MTLS)? {
            for (_, _, body) in list.chunks() {
                let m = Reader::new(body);
                let mut main = need(&m, MTL_MAIN, "name")?;
                let id = main.r_u32()?;
                let name = text.r_str(&mut main)?;
                let desc = match m.sub(MTL_DESC)? {
                    Some(mut d) => text.r_str(&mut d)?,
                    None => SmolStr::default(),
                };
                let flags = need(&m, MTL_FLAGS, "flags")?.r_u32()?;
                let mut ph = need(&m, MTL_PHYSICS, "physics")?;
                let mut fa = need(&m, MTL_FACTORS, "factors")?;
                let opt =
                    |id: u32| -> Result<Option<f32>> { Ok(m.sub(id)?.map(|mut c| c.r_f32()).transpose()?) };
                let mut mat = GameMaterial {
                    id,
                    name,
                    desc,
                    flags,
                    friction: ph.r_f32()?,
                    damping: ph.r_f32()?,
                    spring: ph.r_f32()?,
                    bounce_start_velocity: ph.r_f32()?,
                    bouncing: ph.r_f32()?,
                    shoot_factor: fa.r_f32()?,
                    bounce_damage_factor: fa.r_f32()?,
                    vis_transparency_factor: fa.r_f32()?,
                    snd_occlusion_factor: fa.r_f32()?,
                    shoot_factor_mp: 0.0,
                    flotation_factor: opt(MTL_FLOTATION)?,
                    injurious_speed: opt(MTL_INJURIOUS)?,
                    density_factor: opt(MTL_DENSITY)?,
                };
                mat.shoot_factor_mp = opt(MTL_FACTORS_MP)?.unwrap_or(mat.shoot_factor);
                out.materials.push(mat);
            }
        }
        if let Some(list) = r.sub(CHUNK_MTLS_PAIR)? {
            for (_, _, body) in list.chunks() {
                let p = Reader::new(body);
                let mut pr = need(&p, PAIR_PAIR, "pair")?;
                let (mtl0, mtl1, id, id_parent, own_props) =
                    (pr.r_u32()?, pr.r_u32()?, pr.r_u32()?, pr.r_u32()?, pr.r_u32()?);
                let breaking_sounds = text.r_str(&mut need(&p, PAIR_BREAKING, "breaking sounds")?)?;
                let step_sounds = text.r_str(&mut need(&p, PAIR_STEP, "step sounds")?)?;
                let mut c = need(&p, PAIR_COLLIDE, "collision effects")?;
                let collide_sounds = text.r_str(&mut c)?;
                let collide_particles = text.r_str(&mut c)?;
                let collide_marks = text.r_str(&mut c)?;
                out.pairs.push(GameMaterialPair {
                    mtl0,
                    mtl1,
                    id,
                    id_parent,
                    own_props,
                    breaking_sounds,
                    step_sounds,
                    collide_sounds,
                    collide_particles,
                    collide_marks,
                });
            }
        }
        out.encoding = text.encoding();
        Ok(out)
    }

    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&GameMaterial> {
        self.materials.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    #[must_use]
    pub fn by_id(&self, id: u32) -> Option<&GameMaterial> {
        self.materials.iter().find(|m| m.id == id)
    }

    /// The pair of two materials, either way round.
    #[must_use]
    pub fn pair(&self, a: u32, b: u32) -> Option<&GameMaterialPair> {
        self.pairs.iter().find(|p| (p.mtl0 == a && p.mtl1 == b) || (p.mtl0 == b && p.mtl1 == a))
    }
}

#[cfg(test)]
mod tests {
    use gz_chunk::Writer;

    use super::*;

    #[test]
    fn materials_and_pairs_are_read() {
        let mut w = Writer::new();
        w.chunk(CHUNK_VERSION, |w| w.w_u16(1));
        w.chunk(CHUNK_AUTOINC, |w| {
            w.w_u32(2);
            w.w_u32(1);
        });
        w.chunk(CHUNK_MTLS, |w| {
            w.chunk(0, |w| {
                w.chunk(MTL_MAIN, |w| {
                    w.w_u32(0);
                    w.w_stringz("materials\\wood");
                });
                w.chunk(MTL_FLAGS, |w| w.w_u32(GameMaterial::DYNAMIC));
                w.chunk(MTL_PHYSICS, |w| (0..5).for_each(|i| w.w_f32(i as f32)));
                w.chunk(MTL_FACTORS, |w| (0..4).for_each(|_| w.w_f32(0.5)));
                w.chunk(MTL_DENSITY, |w| w.w_f32(7.0));
            });
        });
        w.chunk(CHUNK_MTLS_PAIR, |w| {
            w.chunk(0, |w| {
                w.chunk(PAIR_PAIR, |w| [0u32, 0, 5, u32::MAX, 0].iter().for_each(|v| w.w_u32(*v)));
                w.chunk(PAIR_BREAKING, |w| w.w_stringz(""));
                w.chunk(PAIR_STEP, |w| w.w_stringz("step\\wood"));
                w.chunk(PAIR_COLLIDE, |w| {
                    w.w_stringz("a");
                    w.w_stringz("b");
                    w.w_stringz("c");
                });
            });
        });
        let m = GameMaterials::read(&w.into_bytes()).unwrap();
        let wood = m.by_name("Materials\\Wood").unwrap();
        assert!(wood.is_dynamic());
        assert_eq!(
            (wood.spring, wood.shoot_factor_mp, wood.density_factor, wood.flotation_factor),
            (2.0, 0.5, Some(7.0), None)
        );
        assert_eq!(m.pair(0, 0).map(|p| p.step_sounds.as_str()), Some("step\\wood"));
        let mut bad = Writer::new();
        bad.chunk(CHUNK_VERSION, |w| w.w_u16(2));
        assert!(GameMaterials::read(&bad.into_bytes()).is_err());
    }
}
