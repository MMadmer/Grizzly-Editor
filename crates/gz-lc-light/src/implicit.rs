//! The implicit light: a
//! texture marked "implicitly lighted" (terrain) is lit texel by texel through its own texture
//! coordinates on every face that wears it without a deflector; the hemisphere goes into the
//! texture's alpha and the light into a lightmap of the texture's size (`<texture>_lm`).

use glam::Vec2;
use gz_fmt_tex::{DdsEncoding, Image, encode_dds};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::bake::{LitScene, Sample};
use crate::batch::{self, BATCH, Gathered, light_samples};
use crate::color::{BaseColor, fixed16_r};
use crate::data::{BuildMaterial, BuildTexture, QUALITY_DRAFT, ShaderXrlc, Shaders, shader_of};
use crate::deflector::{LightEnv, jitter};
use crate::hash2d::Hash2D;
use crate::layer::{LmLayer, apply_borders, image_of, pixels_of};
use crate::math::{Box2, from_bary, normalize, subst_alpha, u8_clr};
use crate::mesh::{Face, FaceId, Mesh};
use crate::ray::RayModel;
use crate::{Ctl, LcError, Result};

const HASH_SIDE: usize = 384;

/// What the implicit light makes of one texture.
#[derive(Clone, Debug, PartialEq)]
pub struct ImplicitMap {
    /// The texture's index.
    pub texture: u32,
    /// The texture's name: the level gets `<name>.dds` and `<name>_lm.dds`.
    pub name: SmolStr,
    /// The texture with the hemisphere in its alpha.
    pub base: Image,
    /// The static light and the sun (in alpha).
    pub lmap: Image,
}

impl ImplicitMap {
    /// Both pictures as DXT5 without mip levels.
    pub fn dds(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            encode_dds(std::slice::from_ref(&self.base), DdsEncoding::Dxt5)?,
            encode_dds(std::slice::from_ref(&self.lmap), DdsEncoding::Dxt5)?,
        ))
    }
}

/// A rendered face whose texture is implicitly lighted.
#[must_use]
pub fn has_implicit_lighting(
    face: &Face,
    shaders: &Shaders,
    materials: &[BuildMaterial],
    textures: &[BuildTexture],
) -> bool {
    if shader_of(face.material, shaders, materials).flags & ShaderXrlc::RENDERING == 0 {
        return false;
    }
    let Some(m) = materials.get(usize::from(face.material)) else { return false };
    textures.get(usize::from(m.surface)).is_some_and(BuildTexture::implicit_lighted)
}

/// What the implicit light needs besides the mesh and the textures it changes.
#[derive(Clone, Copy, Debug)]
pub struct ImplicitInput<'a> {
    pub model: &'a RayModel,
    pub shaders: &'a Shaders,
    pub materials: &'a [BuildMaterial],
    pub lights: &'a crate::light::LightSet,
    pub params: &'a crate::data::BuildParams,
    pub nosun: bool,
    pub linear: bool,
}

/// Every implicitly lighted texture, in the order of its index, lit through `scene`; each face
/// it was found on gets a second channel of its coordinates (`Initialize`, "compatible with
/// lightmaps"). A map's `base` is the texture with the hemisphere in its alpha: what
/// [`apply_implicit`] makes the texture's surface. Nothing at the draft quality.
pub fn implicit_lighting(
    mesh: &mut Mesh,
    textures: &[BuildTexture],
    input: &ImplicitInput<'_>,
    scene: &dyn LitScene,
    ctl: &Ctl<'_>,
) -> Result<Vec<ImplicitMap>> {
    if input.params.quality == QUALITY_DRAFT {
        return Ok(Vec::new());
    }
    ctl.stage("sorting the faces");
    let mut by_texture: std::collections::BTreeMap<u32, Vec<FaceId>> = std::collections::BTreeMap::new();
    for &f in mesh.faces() {
        let Some(face) = mesh.face(f) else { continue };
        if face.deflector.is_some() || !has_implicit_lighting(face, input.shaders, input.materials, textures)
        {
            continue;
        }
        let tid = input.materials.get(usize::from(face.material)).map_or(0, |m| u32::from(m.surface));
        by_texture.entry(tid).or_default().push(f);
    }

    let mut out = Vec::with_capacity(by_texture.len());
    for (tid, faces) in by_texture {
        ctl.check()?;
        let Some(tex) = textures.get(tid as usize) else { continue };
        ctl.stage(&format!("the texture {}", tex.name));
        let Some(surface) = tex.surface.as_ref() else {
            return Err(LcError::Invalid(format!(
                "The implicitly lighted texture '{}' has no surface.",
                tex.name
            )));
        };
        for &f in &faces {
            if let Some(face) = mesh.face_mut(f) {
                let tc = face.tc0();
                face.add_channel(tc[0], tc[1], tc[2]);
            }
        }
        let env = LightEnv {
            mesh,
            model: input.model,
            textures,
            lights: input.lights,
            params: input.params,
            nosun: input.nosun,
            linear: input.linear,
            shrink: true,
            second_page_only: false,
        };
        let mut lmap = light_texture(&env, tex.width, tex.height, &faces, scene, ctl)?;
        for r in (1..=254).rev() {
            if !apply_borders(&mut lmap, r) {
                break;
            }
        }
        let (w, h) = (tex.width, tex.height);
        let with_hemi: Vec<u32> = surface
            .par_iter()
            .zip(lmap.surface.par_iter())
            .map(|(&c, l)| subst_alpha(c, u8_clr(fixed16_r(l.h))))
            .collect();
        let base = image_of(w, h, &with_hemi);
        let lm = image_of(w, h, &lmap.pack());
        out.push(ImplicitMap { texture: tid, name: tex.name.clone(), base, lmap: lm });
    }
    Ok(out)
}

/// Every lit texture's surface takes the hemisphere in its alpha, as its map's `base` holds it.
pub fn apply_implicit(textures: &mut [BuildTexture], maps: &[ImplicitMap]) {
    for m in maps {
        if let Some(t) = textures.get_mut(m.texture as usize)
            && let Some(surface) = t.surface.as_mut()
        {
            let pixels = pixels_of(&m.base);
            if pixels.len() == surface.len() {
                *surface = pixels;
            }
        }
    }
}

/// Whether a ray of `model` reads the alpha of a texture the maps change: a transparent face that
/// casts a shadow wears one. Then what is lit after the implicit light sees the new alpha.
#[must_use]
pub fn implicit_shadows(model: &RayModel, maps: &[ImplicitMap]) -> bool {
    !maps.is_empty()
        && model
            .faces()
            .iter()
            .any(|f| f.cast_shadow && !f.opaque && maps.iter().any(|m| m.texture == f.texture))
}

/// Executes the implicit light over every row: a texel takes the light of each of its jittered
/// samples on every face that holds it. The rows of a batch are gathered in parallel.
pub(crate) fn light_texture(
    env: &LightEnv<'_>,
    w: u32,
    h: u32,
    faces: &[FaceId],
    scene: &dyn LitScene,
    ctl: &Ctl<'_>,
) -> Result<LmLayer> {
    let mut lmap = LmLayer::new(w, h);
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 {
        return Ok(lmap);
    }
    let mesh = env.mesh;
    let tcs: Vec<[Vec2; 3]> =
        faces.iter().map(|&f| mesh.face(f).map_or([Vec2::ZERO; 3], Face::tc0)).collect();
    let mut bounds = Box2::INVALID;
    let boxes: Vec<Box2> = tcs.iter().map(Box2::of_tri).collect();
    boxes.iter().for_each(|b| bounds.merge(b));
    let hash = Hash2D::build(HASH_SIDE, HASH_SIDE, bounds, &boxes);
    let world: Vec<Option<([glam::Vec3; 3], [glam::Vec3; 3])>> =
        faces.iter().map(|&f| Some((mesh.positions(f)?, mesh.normals(f)?))).collect();

    let dim = Vec2::new(w as f32, h as f32);
    let half = Vec2::new(0.5 / dim.x, 0.5 / dim.y);
    let js = Vec2::new(0.499 / dim.x, 0.499 / dim.y);
    let jit = jitter(env.params.lm_jitter_samples);
    let flags = env.sun_flag();
    let job = env.job();
    let rows_per = (BATCH / (wu * jit.len()).max(1)).max(1);
    let mut v0 = 0usize;
    while v0 < hu {
        ctl.check()?;
        let v1 = (v0 + rows_per).min(hu);
        let rows: Vec<(Gathered, Vec<u8>)> = (v0..v1)
            .into_par_iter()
            .map(|v| {
                let mut g = Gathered::default();
                let mut marks = vec![0u8; wu];
                for (u, m) in marks.iter_mut().enumerate() {
                    let first = g.begin();
                    for j in jit {
                        let p = Vec2::new(
                            u as f32 / dim.x + half.x + j.x * js.x,
                            v as f32 / dim.y + half.y + j.y * js.y,
                        );
                        for &i in hash.query(p.x, p.y) {
                            let (Some(tc), Some(Some((pos, nrm))), Some(&f)) =
                                (tcs.get(i as usize), world.get(i as usize), faces.get(i as usize))
                            else {
                                continue;
                            };
                            let Some(b) = (crate::mesh::Tcf { uv: *tc }).inside(p) else { continue };
                            let wp = from_bary(pos[0], pos[1], pos[2], b);
                            let wn = normalize(from_bary(nrm[0], nrm[1], nrm[2], b));
                            g.samples.push(Sample::new(wp, wn, flags, Some(f)));
                        }
                    }
                    if g.samples.len() as u32 > first {
                        g.end((v * wu + u) as u32, first);
                        *m = 255;
                    }
                }
                (g, marks)
            })
            .collect();
        let mut all = Gathered::default();
        for (v, (g, marks)) in (v0..v1).zip(rows) {
            all.append(g);
            if let Some(row) = lmap.marker.get_mut(v * wu..(v + 1) * wu) {
                row.copy_from_slice(&marks);
            }
        }
        let lit = light_samples(scene, &job, &all.samples, ctl)?;
        for r in &all.runs {
            let mut c = batch::sum(&lit, r);
            c.scale(r.count);
            c.mul(0.5);
            if let Some(s) = lmap.surface.get_mut(r.item as usize) {
                *s = BaseColor::from_c(&c);
            }
        }
        v0 = v1;
    }
    Ok(lmap)
}
