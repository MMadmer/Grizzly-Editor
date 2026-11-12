//! `CBuild::Load` (`Build_Load.cpp`): `build.prj` read into the compiler's model with the
//! compiler shaders and the textures' descriptions and surfaces; and the same for the detail
//! compiler (`global_claculation_data::xrLoad`).

use std::path::{Path, PathBuf};

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

/// Where the compilers read the editor's data: `$game_data$` (`shaders_xrlc.xr`) and
/// `$game_textures$` (the descriptions and the pictures).
#[derive(Clone, Debug)]
pub struct Sources {
    pub game_data: PathBuf,
    pub textures: PathBuf,
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
        Self { game_data, textures }
    }

    /// `shaders_xrlc.xr`.
    pub fn shaders(&self) -> Result<Shaders> {
        let path = self.game_data.join("shaders_xrlc.xr");
        let bytes = std::fs::read(&path).map_err(|e| {
            Error::Io(format!("The compiler shaders {} could not be read: {e}", path.display()))
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

/// What `CBuild` keeps of `build.prj` besides the model.
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

/// `CBuild::Load`. `smoothing_groups` is off for `-nosmg` (the faces keep no group).
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
    // `FS.r_close(F)` after `CBuild::Load`: the pools' later blocks go where build.prj was
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
