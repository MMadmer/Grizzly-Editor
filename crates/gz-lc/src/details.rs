//! The detail compiler: `build.details` copied into `level.details`, every slot's palette
//! processed and its light taken from `build.cform` and `build.lights` with the build's
//! materials and textures.

use gz_fmt_level::{BuildProject, LevelDetails};
use gz_lc_light::{
    BuildTexture, Ctl, DetailScene, LightSet, RcastForm, Shaders, light_details, post_process_materials,
};

use crate::{Error, Log, Result};

/// The detail light in memory: answers `level.details`.
#[allow(clippy::too_many_arguments)]
pub fn compile_details_in_memory(
    prj: &BuildProject,
    shaders: Shaders,
    textures: Vec<BuildTexture>,
    build_details: &[u8],
    build_cform: &[u8],
    build_lights: &[u8],
    ctl: &Ctl<'_>,
    log: &mut Log,
) -> Result<Vec<u8>> {
    let damaged = |name: &'static str| move |e: String| Error::Invalid(format!("{name} is damaged: {e}"));
    crate::whole_chunks("build.cform", build_cform)?;
    crate::whole_chunks("build.lights", build_lights)?;
    crate::whole_chunks("build.details", build_details)?;
    let form = RcastForm::read(build_cform).map_err(|e| damaged("build.cform")(e.to_string()))?;
    let lights =
        LightSet::read_build_lights(build_lights).map_err(|e| damaged("build.lights")(e.to_string()))?;
    let mut materials = prj.materials.clone();
    post_process_materials(&shaders, &prj.shaders_xrlc, &mut materials)?;
    let mut details =
        LevelDetails::read(build_details).map_err(|e| damaged("build.details")(e.to_string()))?;
    if details.header.version != gz_fmt_level::DETAIL_VERSION {
        return Err(Error::Invalid(format!(
            "build.details has version {}, the compiler reads version {}.",
            details.header.version,
            gz_fmt_level::DETAIL_VERSION
        )));
    }
    let scene = DetailScene::new(&form, shaders, materials, textures, lights, details.header)?;
    log.msg(format!("* Level CFORM: {} faces", form.tris.len()));
    let mut slots = details.slots.to_vec();
    let stats = light_details(&scene, &mut slots, None, ctl)?;
    log.msg(format!("{} slots lit, {} empty, {} without ground", stats.lit, stats.empty, stats.no_ground));
    details.slots = slots.into();
    Ok(details.write())
}
