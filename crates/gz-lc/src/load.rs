//! The load: `build.prj` read into the compiler's model with the
//! compiler shaders and the textures' descriptions and surfaces; and the same for the detail
//! compiler.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_fmt_level::{
    Aabb, BuildGlow, BuildLightDynamic, BuildLod, BuildProject, Portal, XRCL_CURRENT_VERSION,
};
use gz_fmt_tex::{Image, TextureParams, load_image, read_texture_thm};
use gz_fmt_xr::ShadersXrlc;
use gz_lc_light::{
    BuildTexture, LcGlobalData, LightSet, Shaders, TextureUse, post_process_materials, prepare_texture,
    texture_key,
};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::{Error, Result};

/// A texture by its name, from somewhere other than the textures folder: the project's texture
/// asset that publishes the name (`docs/ASSETS.md` section 6), its description and its picture.
/// Asked first; `None` reads the folder.
#[derive(Clone)]
pub struct AssetTextures(pub Arc<AssetTexture>);

/// One texture asset by its name: its description and its picture.
pub type AssetTexture = dyn Fn(&str) -> Option<(TextureParams, Image)> + Send + Sync;

impl std::fmt::Debug for AssetTextures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssetTextures")
    }
}

/// Where the compilers read the editor's data: `$game_data$` (`shaders_xrlc.xr`) and
/// `$game_textures$` (the descriptions and the pictures), and the textures the project's assets
/// give, before that folder.
#[derive(Clone, Debug)]
pub struct Sources {
    pub game_data: PathBuf,
    pub textures: PathBuf,
    pub assets: Option<AssetTextures>,
}

impl Sources {
    /// The data under an editor root, as its `fsgame.ltx` names them when it can be read
    /// (`$game_data$`, `$game_textures$`), else `gamedata` and `gamedata\textures`.
    #[must_use]
    pub fn of_root(root: &Path) -> Self {
        let mut game_data = root.join("gamedata");
        let mut textures = game_data.join("textures");
        if let Ok(bytes) = std::fs::read(root.join("fsgame.ltx")) {
            let text = gz_base::text::cp1251_to_utf8(&bytes);
            let mut aliases: Vec<(String, PathBuf)> = vec![("$fs_root$".into(), root.to_path_buf())];
            for line in text.lines() {
                let line = line.split(';').next().unwrap_or_default();
                let Some((name, rest)) = line.split_once('=') else { continue };
                let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
                let (Some(base), add) = (parts.get(2), parts.get(3)) else { continue };
                let Some(base) =
                    aliases.iter().find(|(a, _)| a.eq_ignore_ascii_case(base)).map(|x| x.1.clone())
                else {
                    continue;
                };
                let p = match add {
                    Some(a) if !a.is_empty() => base.join(a.trim_end_matches(['\\', '/'])),
                    _ => base,
                };
                aliases.push((name.trim().to_owned(), p));
            }
            let find =
                |n: &str| aliases.iter().rev().find(|(a, _)| a.eq_ignore_ascii_case(n)).map(|x| x.1.clone());
            if let Some(p) = find("$game_data$") {
                game_data = p;
            }
            if let Some(p) = find("$game_textures$") {
                textures = p;
            }
        }
        Self { game_data, textures, assets: None }
    }

    /// The same, with the textures of `assets` asked first.
    #[must_use]
    pub fn with_assets(self, assets: AssetTextures) -> Self {
        Self { assets: Some(assets), ..self }
    }

    /// `shaders_xrlc.xr`.
    pub fn shaders(&self) -> Result<Shaders> {
        let path = self.game_data.join("shaders_xrlc.xr");
        let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::Io(format!("The compiler shaders {} are not there.", path.display()))
            }
            _ => Error::Io(format!("The compiler shaders {} could not be read: {e}", path.display())),
        })?;
        let list =
            ShadersXrlc::read(&bytes).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        Ok(Shaders::new(list.shaders))
    }

    fn thm(&self, key: &str) -> Option<TextureParams> {
        let bytes = std::fs::read(self.textures.join(format!("{key}.thm"))).ok()?;
        read_texture_thm(&bytes).ok().map(|t| t.params)
    }

    fn surface(&self, key: &str) -> Option<Image> {
        let bytes = std::fs::read(self.textures.join(format!("{key}.dds"))).ok()?;
        load_image(&bytes, Some("dds")).ok().map(|l| l.image)
    }

    /// The textures of a build prepared for `usage`, read in parallel.
    pub fn textures(
        &self,
        prj_textures: &[gz_fmt_level::BuildTexture],
        usage: TextureUse,
    ) -> Result<Vec<BuildTexture>> {
        prj_textures
            .par_iter()
            .map(|raw| {
                let key = texture_key(&raw.name);
                if key != "level_lods"
                    && let Some((params, image)) = self.assets.as_ref().and_then(|a| (a.0)(&key))
                {
                    let mut picture = Some(image);
                    return prepare_texture(raw, Some(&params), usage, |k| {
                        if k == key { picture.take() } else { self.surface(k) }
                    })
                    .map_err(Error::from);
                }
                let thm = if key == "level_lods" { None } else { self.thm(&key) };
                if key != "level_lods" && thm.is_none() {
                    return Err(Error::Invalid(format!(
                        "The description of the texture '{key}' ({}) could not be read.",
                        self.textures.join(format!("{key}.thm")).display()
                    )));
                }
                prepare_texture(raw, thm.as_ref(), usage, |k| self.surface(k)).map_err(Error::from)
            })
            .collect()
    }
}

/// What the compiler keeps of `build.prj` besides the model.
#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub scene_bb: Aabb,
    pub shader_render: Vec<SmolStr>,
    pub shader_compile: Vec<SmolStr>,
    pub glows: Vec<BuildGlow>,
    pub portals: Vec<Portal>,
    pub lods: Vec<BuildLod>,
    pub lights_dynamic: Vec<BuildLightDynamic>,
}

/// Loads the level. `smoothing_groups` is off for `-nosmg` (the faces keep no group).
pub fn load(
    prj: &BuildProject,
    shaders: Shaders,
    textures: Vec<BuildTexture>,
    smoothing_groups: bool,
) -> Result<(LcGlobalData, Extra)> {
    if prj.version != XRCL_CURRENT_VERSION {
        return Err(Error::Invalid(format!(
            "build.prj has version {}, the compiler reads version {XRCL_CURRENT_VERSION}.",
            prj.version
        )));
    }
    if smoothing_groups && prj.smooth_groups.len() != prj.faces.len() {
        return Err(Error::Invalid(
            "build.prj has no smoothing group for every face (EB_SmoothGroups).".into(),
        ));
    }
    let lights = LightSet::from_project(&prj.light_controls, &prj.lights_static)?;
    if lights.hemi.is_empty() || lights.sun.is_empty() {
        return Err(Error::Invalid("The level has no hemisphere or no sun light ($hemi, $sun).".into()));
    }
    let mut materials = prj.materials.clone();
    post_process_materials(&shaders, &prj.shaders_xrlc, &mut materials)?;
    let mut g = LcGlobalData::new(prj.params, shaders, materials, textures, lights);
    g.load_mesh(prj)?;
    if !smoothing_groups {
        let faces = g.mesh.faces().to_vec();
        for f in faces {
            if let Some(face) = g.mesh.face_mut(f) {
                face.sm_group = gz_lc_light::NO_SM_GROUP;
            }
        }
    }
    g.load_mu(prj)?;
    // a piece's occluders: what the rays read of them, a transparent one by its texture's alpha
    g.occluders = prj
        .occluders
        .iter()
        .map(|o| gz_lc_light::Occluder {
            p: o.p,
            texture: u32::from(o.texture),
            uv: o.t,
            opaque: g.textures.get(usize::from(o.texture)).is_none_or(|t| !(t.has_alpha && t.has_surface)),
        })
        .collect();
    // the pools' later blocks go where build.prj was
    g.mesh.mark_loaded();
    let mut bb = Aabb::INVALID;
    for p in prj.verts.iter() {
        bb.modify(*p);
    }
    let extra = Extra {
        scene_bb: bb,
        shader_render: prj.shaders.clone(),
        shader_compile: prj.shaders_xrlc.clone(),
        glows: prj.glows.clone(),
        portals: prj.portals.clone(),
        lods: prj.lods.clone(),
        lights_dynamic: prj.lights_dynamic.clone(),
    };
    Ok((g, extra))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use gz_fmt_tex::TextureFormat;
    use gz_lc_light::{CpuLighter, Ctl};

    use super::*;

    /// A texture asset's description and its 4 x 4 picture, with alpha: the compiler reads its
    /// surface.
    fn asset(key: &str) -> Option<(TextureParams, Image)> {
        let params =
            TextureParams { width: 64, height: 64, fmt: TextureFormat::DXT5, ..TextureParams::default() };
        let rgba = if key == "grnd\\floor" { [10, 200, 30, 255] } else { [90, 90, 90, 128] };
        Some((params, Image::filled(4, 4, rgba).ok()?))
    }

    #[test]
    fn a_texture_asset_is_asked_before_the_textures_folder_and_the_level_compiles_with_it() {
        let (prj, shaders, _) = crate::tests::project();
        let nowhere = std::env::temp_dir().join(format!("gz-lc-assets-{}", std::process::id()));
        let plain = Sources { game_data: nowhere.clone(), textures: nowhere.join("textures"), assets: None };
        // the folder has no description of either texture
        let refused = plain.textures(&prj.textures, TextureUse::Level { radiosity: false }).unwrap_err();
        assert!(refused.to_string().contains("The description of the texture"), "{refused}");

        let sources = plain.with_assets(AssetTextures(Arc::new(asset)));
        let textures = sources.textures(&prj.textures, TextureUse::Level { radiosity: false }).unwrap();
        assert_eq!(textures.len(), 2);
        for t in &textures {
            assert!(t.has_surface && t.has_alpha, "{}", t.name);
            // the picture's own size, not the description's
            assert_eq!((t.width, t.height, t.thm.width), (4, 4, 4));
        }
        let floor = textures[0].surface.as_ref().unwrap();
        assert_eq!(floor.len(), 16);
        let mut log = crate::Log::default();
        let out = crate::compile_project(
            &prj,
            shaders,
            textures,
            &crate::Options::default(),
            &CpuLighter::default(),
            &Ctl::quiet(),
            &mut log,
        )
        .unwrap();
        assert!(out.files.iter().any(|(n, _)| n == "level.geom"));
    }
}
