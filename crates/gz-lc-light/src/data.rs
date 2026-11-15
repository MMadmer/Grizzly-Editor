//! What a build lights with: the compiler shaders, the materials
//! (`b_material`, `post_process_materials`, `shader()`) and the textures with their surfaces
//! (`b_BuildTexture`, the texture part of the level load).

use gz_fmt_tex::{Image, TextureFlags, TextureFormat, TextureParams};
use smol_str::SmolStr;

use crate::math::color_rgba;
use crate::{LcError, Result};

pub use gz_fmt_level::{BuildMaterial, BuildParams};
pub use gz_fmt_xr::ShaderXrlc;

/// The draft quality.
pub const QUALITY_DRAFT: u16 = 0;
/// `b_material::shader_xrlc` of a material without a compiler shader.
pub const NO_SHADER: u16 = 65535;

/// A bit of `b_material::internal_max_area`, which no compiler reads otherwise: the material's
/// faces keep the light the level they lie in has baked into its own texture. Set by the build of
/// a module's piece of a level of the game on a terrain's faces whose look is the level's ground;
/// where the texture is implicitly lighted, such a face is not lit again and its visual names the
/// level's own `<texture>` and `<texture>_lm`, which the game finds in the level.
pub const KEEPS_LEVEL_LIGHT: u32 = 1 << 31;

/// Whether a material's faces keep the level's own light ([`KEEPS_LEVEL_LIGHT`]).
#[must_use]
pub fn keeps_level_light(material: u16, materials: &[BuildMaterial]) -> bool {
    materials.get(usize::from(material)).is_some_and(|m| m.internal_max_area & KEEPS_LEVEL_LIGHT != 0)
}

/// What a shader is before the library says otherwise.
#[must_use]
pub fn default_shader() -> ShaderXrlc {
    let mut name_raw = [0u8; 128];
    name_raw[..7].copy_from_slice(b"unknown");
    ShaderXrlc {
        name_raw,
        flags: ShaderXrlc::COLLISION
            | ShaderXrlc::RENDERING
            | ShaderXrlc::OPTIMIZE_UV
            | ShaderXrlc::LIGHT_CAST_SHADOW
            | ShaderXrlc::LIGHT_SHARP,
        vert_translucency: 0.5,
        vert_ambient: 0.0,
        lm_density: 1.0,
    }
}

/// The compiler shaders of `shaders_xrlc.xr`.
#[derive(Clone, Debug)]
pub struct Shaders {
    pub list: Vec<ShaderXrlc>,
    fallback: ShaderXrlc,
}

impl Default for Shaders {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Shaders {
    #[must_use]
    pub fn new(list: Vec<ShaderXrlc>) -> Self {
        Self { list, fallback: default_shader() }
    }

    /// The index of a shader by its name, case ignored.
    #[must_use]
    pub fn id_of(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|s| s.name().eq_ignore_ascii_case(name))
    }

    /// The material by id. X-Ray's compiler indexes past the library for a material without a compiler shader
    /// (`reserved = 65535`); here such a material has the default shader.
    #[must_use]
    pub fn get(&self, id: u16) -> &ShaderXrlc {
        self.list.get(usize::from(id)).unwrap_or(&self.fallback)
    }
}

/// `post_process_materials`: every material's `reserved` becomes the index of its compiler
/// shader in the library, 65535 for none.
pub fn post_process_materials(
    shaders: &Shaders,
    shader_compile: &[SmolStr],
    materials: &mut [BuildMaterial],
) -> Result<()> {
    for m in materials.iter_mut() {
        if m.shader_xrlc == NO_SHADER {
            m.reserved = u16::MAX;
            continue;
        }
        let name = shader_compile.get(usize::from(m.shader_xrlc)).map_or("", SmolStr::as_str);
        let Some(id) = shaders.id_of(name) else {
            return Err(LcError::Invalid(format!(
                "The shader '{name}' is not in the compiler's shader library."
            )));
        };
        m.reserved = id as u16;
    }
    Ok(())
}

/// `shader(dwMaterial, shaders, materials)`.
#[must_use]
pub fn shader_of<'a>(material: u16, shaders: &'a Shaders, materials: &[BuildMaterial]) -> &'a ShaderXrlc {
    let id = materials.get(usize::from(material)).map_or(u16::MAX, |m| m.reserved);
    shaders.get(id)
}

/// The texture's alpha channel as the compiler has it (no BC7).
#[must_use]
pub fn has_alpha_channel(fmt: TextureFormat) -> bool {
    matches!(
        fmt,
        TextureFormat::ADXT1
            | TextureFormat::DXT3
            | TextureFormat::DXT5
            | TextureFormat::RGBA4444
            | TextureFormat::RGBA1555
            | TextureFormat::RGBA
    )
}

/// `b_BuildTexture`: a texture of the build, with its pixels when the light needs them
/// (`0xAARRGGBB`, rows top-down as the image file holds them unless flipped).
#[derive(Clone, Debug, Default)]
pub struct BuildTexture {
    pub name: SmolStr,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub surface: Option<Vec<u32>>,
    /// The surface was loaded (it may have been dropped since).
    pub has_surface: bool,
    pub thm: TextureParams,
}

impl BuildTexture {
    /// The texel at `x`, `y`; none outside the picture or without its pixels.
    #[must_use]
    pub fn texel(&self, x: u32, y: u32) -> Option<u32> {
        let s = self.surface.as_ref()?;
        s.get(y as usize * self.width as usize + x as usize).copied()
    }

    /// Flips a picture vertically.
    pub fn vflip(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        let Some(s) = self.surface.as_mut() else { return };
        if s.len() < w * h {
            return;
        }
        for y in 0..h / 2 {
            let y2 = h - y - 1;
            let (top, bottom) = s.split_at_mut(y2 * w);
            if let (Some(a), Some(b)) = (top.get_mut(y * w..y * w + w), bottom.get_mut(..w)) {
                a.swap_with_slice(b);
            }
        }
    }

    /// `implicit lighted` in the description.
    #[must_use]
    pub fn implicit_lighted(&self) -> bool {
        self.thm.flags.contains(TextureFlags::IMPLICIT_LIGHTED)
    }
}

/// The pixels of a picture as the compiler holds them (`0xAARRGGBB`).
#[must_use]
pub fn surface_of(image: &Image) -> Vec<u32> {
    image.rgba.as_chunks::<4>().0.iter().map(|p| color_rgba(p[0], p[1], p[2], p[3])).collect()
}

/// The name a texture is looked up by: up to its first dot, in lower case.
#[must_use]
pub fn texture_key(name: &str) -> String {
    name.split('.').next().unwrap_or("").to_lowercase()
}

/// Which light a texture is prepared for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureUse {
    /// `radiosity` loads every surface; rows stay as the file has them.
    Level { radiosity: bool },
    /// The surface is flipped.
    Details,
}

/// A texture of `build.prj` made ready for the light: the merged LOD texture by its fixed size,
/// any other by its description (`.thm`), and its surface loaded through `load` (given the
/// texture key) when it has alpha or is implicitly lighted.
pub fn prepare_texture(
    raw: &gz_fmt_level::BuildTexture,
    thm: Option<&TextureParams>,
    usage: TextureUse,
    load: impl FnOnce(&str) -> Option<Image>,
) -> Result<BuildTexture> {
    let key = texture_key(&raw.name);
    let mut bt = BuildTexture {
        name: SmolStr::new(&key),
        width: raw.width,
        height: raw.height,
        has_alpha: raw.has_alpha != 0,
        ..BuildTexture::default()
    };
    if key == "level_lods" {
        // the merged LOD texture has no description of its own
        bt.width = 1024;
        bt.height = 1024;
        bt.has_alpha = true;
        return Ok(bt);
    }
    let Some(thm) = thm else {
        return Err(LcError::Invalid(format!("The texture '{key}' has no description (.thm).")));
    };
    bt.thm = thm.clone();
    bt.width = thm.width.max(0) as u32;
    bt.height = thm.height.max(0) as u32;
    bt.has_alpha = has_alpha_channel(thm.fmt);
    let lod = key.starts_with("lod\\");
    let radiosity = matches!(usage, TextureUse::Level { radiosity: true });
    if lod || !(bt.has_alpha || bt.implicit_lighted() || radiosity) {
        return Ok(bt);
    }
    let Some(image) = load(&key) else {
        return Err(LcError::Invalid(format!("The surface of the texture '{key}' could not be loaded.")));
    };
    // the detail compiler of X-Ray keeps the description's size here and reads past a smaller
    // surface; both compilers take the surface's own size
    bt.width = image.width;
    bt.height = image.height;
    bt.thm.width = image.width as i32;
    bt.thm.height = image.height as i32;
    bt.surface = Some(surface_of(&image));
    bt.has_surface = true;
    if usage == TextureUse::Details {
        bt.vflip();
    }
    Ok(bt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_material_finds_its_compiler_shader_or_none() {
        let mut a = default_shader();
        a.name_raw = [0; 128];
        a.name_raw[..4].copy_from_slice(b"wall");
        let shaders = Shaders::new(vec![default_shader(), a]);
        let mut mats = vec![
            BuildMaterial { shader_xrlc: 0, ..BuildMaterial::default() },
            BuildMaterial { shader_xrlc: NO_SHADER, ..BuildMaterial::default() },
        ];
        post_process_materials(&shaders, &["WALL".into()], &mut mats).unwrap();
        assert_eq!(mats[0].reserved, 1);
        assert_eq!(mats[1].reserved, u16::MAX);
        assert_eq!(shader_of(1, &shaders, &mats).name(), "unknown");
        let missing = post_process_materials(&shaders, &["nope".into()], &mut mats[..1]);
        assert!(missing.is_err());
    }

    #[test]
    fn a_detail_texture_is_flipped_and_sized_by_its_surface() {
        let raw = gz_fmt_level::BuildTexture {
            name: "Grass.dds".into(),
            width: 8,
            height: 8,
            has_alpha: 0,
            reserve: 0,
        };
        let thm = TextureParams { fmt: TextureFormat::DXT5, width: 8, height: 8, ..TextureParams::default() };
        let img = Image::new(1, 2, vec![1, 2, 3, 10, 4, 5, 6, 20]).unwrap();
        let t = prepare_texture(&raw, Some(&thm), TextureUse::Details, |k| {
            assert_eq!(k, "grass");
            Some(img.clone())
        })
        .unwrap();
        assert_eq!((t.width, t.height, t.has_alpha), (1, 2, true));
        assert_eq!(t.texel(0, 0), Some(color_rgba(4, 5, 6, 20)));
        let lods = gz_fmt_level::BuildTexture { name: "level_lods".into(), ..raw };
        let t = prepare_texture(&lods, None, TextureUse::Level { radiosity: false }, |_| None).unwrap();
        assert_eq!((t.width, t.has_alpha, t.surface.is_none()), (1024, true, true));
    }
}
