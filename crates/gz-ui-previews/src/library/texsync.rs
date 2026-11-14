//! What the Texture Library writes: a description (`.thm`) beside its source
//! picture in `rawdata\textures`, the game texture (`.dds`) in `gamedata\textures`, and - for
//! New Textures - the source copied over from the import folder. Blocking, for jobs.

use std::path::{Path, PathBuf};

use gz_fmt_tex::{
    Image, TextureParams, TextureThm, TextureType, TgaDepth, is_valid_size, load_image, make_game_texture,
    read_texture_thm, write_texture_thm, write_tga,
};

use crate::paths::{EditorDirs, file_age, set_file_age, texture_folder_name};

/// The seconds since 1970 now: the age a synchronised file gets.
#[must_use]
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn rel(name: &str, ext: &str) -> String {
    format!("{}.{ext}", name.replace('/', "\\"))
}

/// The description of a library texture: its `.thm`, `None` when it has none or it does not
/// read.
#[must_use]
pub fn read_thm(dir: &Path, name: &str) -> Option<TextureThm> {
    std::fs::read(dir.join(rel(name, "thm"))).ok().and_then(|b| read_texture_thm(&b).ok())
}

/// A source picture: `.tga` and every other picture the decoder reads.
pub fn read_picture(path: &Path) -> Result<(Image, bool), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let loaded = load_image(&bytes, ext.as_deref())
        .map_err(|e| format!("{} is no picture the editor reads: {e}", path.display()))?;
    Ok((loaded.image, loaded.has_alpha))
}

/// What one texture's synchronisation did.
#[derive(Debug, Default)]
pub struct Synced {
    pub thm: bool,
    pub game: bool,
    /// The bump map's gloss mask is under 10 percent ("Invalid gloss mask").
    pub low_gloss: bool,
}

/// Writes a texture's description and its game texture: `params` are
/// the edited ones, `None` keeps those on disk (or makes the defaults of a new picture).
pub fn sync_texture(
    dirs: &EditorDirs,
    name: &str,
    params: Option<&TextureParams>,
    write_thm: bool,
    write_game: bool,
    age: i64,
) -> Result<Synced, String> {
    let tga = dirs.textures.join(rel(name, "tga"));
    let (image, alpha) = read_picture(&tga)?;
    let old = read_thm(&dirs.textures, name);
    let mut params = match (params, &old) {
        (Some(p), _) => p.clone(),
        (None, Some(t)) => t.params.clone(),
        (None, None) => TextureParams::for_new_texture(image.width, image.height, alpha),
    };
    params.width = i32::try_from(image.width).unwrap_or(i32::MAX);
    params.height = i32::try_from(image.height).unwrap_or(i32::MAX);
    params.flags.set(gz_fmt_tex::TextureFlags::HAS_ALPHA, alpha);
    let mut out = Synced::default();
    let thm_path = dirs.textures.join(rel(name, "thm"));
    if write_thm {
        let thm = TextureThm { pixels: old.and_then(|t| t.pixels), params: params.clone() };
        let bytes = write_texture_thm(&thm)
            .map_err(|e| format!("the description of {name} could not be made: {e}"))?;
        gz_editor::fsx::write_atomic(&thm_path, &bytes)
            .map_err(|e| format!("{} could not be written: {e}", thm_path.display()))?;
        out.thm = true;
    }
    let dds_path = dirs.game_textures.join(rel(name, "dds"));
    if write_game {
        if !is_valid_size(image.width, image.height) {
            return Err(format!(
                "Can't make game texture '{name}'. Invalid size ({}x{}).",
                image.width, image.height
            ));
        }
        // a bump map may name a normal map of its own, a 32-bit normal map of the same size
        let normal = if params.ty == TextureType::BUMP_MAP && !params.ext_normal_map_name.is_empty() {
            let nm = params.ext_normal_map_name.to_string();
            let path = dirs.game_textures.join(rel(&nm, "dds"));
            let (img, _) =
                read_picture(&path).map_err(|_| format!("Can't load special normal map texture '{nm}'."))?;
            if img.width != image.width || img.height != image.height {
                return Err(format!(
                    "Invalid load special normal map size '{nm}'. It should be [{}x{}]",
                    image.width, image.height
                ));
            }
            Some(img)
        } else {
            None
        };
        let made = make_game_texture(&image, &params, normal.as_ref())
            .map_err(|e| format!("Can't make game texture '{name}': {e}"))?;
        gz_editor::fsx::write_atomic(&dds_path, &made.dds)
            .map_err(|e| format!("{} could not be written: {e}", dds_path.display()))?;
        let bump = dirs.game_textures.join(format!("{}#.dds", name.replace('/', "\\")));
        match &made.bump_error_dds {
            Some(b) => gz_editor::fsx::write_atomic(&bump, b)
                .map_err(|e| format!("{} could not be written: {e}", bump.display()))?,
            None => {
                let _ = std::fs::remove_file(&bump);
            }
        }
        out.game = true;
        out.low_gloss = made.low_gloss;
    }
    if out.thm || out.game {
        for p in [&tga, &thm_path, &dds_path] {
            if p.is_file() {
                let _ = set_file_age(p, age);
            }
        }
    }
    Ok(out)
}

/// A picture of the import folder taken into the library: copied as a
/// `.tga` under its folder name (`wpn_ak74` into `wpn\`), its age kept, the import file renamed
/// to `.~<ext>`. Returns the library name.
pub fn import_picture(dirs: &EditorDirs, file: &str) -> Result<String, String> {
    let src = dirs.import.join(file);
    let stem = crate::paths::strip_ext(file);
    let name = texture_folder_name(stem);
    let dest = dirs.textures.join(rel(&name, "tga"));
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be made: {e}", dir.display()))?;
    }
    let is_tga = file.to_lowercase().ends_with(".tga");
    if is_tga {
        std::fs::copy(&src, &dest).map_err(|e| format!("{} could not be copied: {e}", src.display()))?;
    } else {
        let (image, alpha) = read_picture(&src)?;
        let tga = write_tga(&image, if alpha { TgaDepth::Bgra32 } else { TgaDepth::Bgr24 })
            .map_err(|e| format!("{file} could not be converted: {e}"))?;
        gz_editor::fsx::write_atomic(&dest, &tga)
            .map_err(|e| format!("{} could not be written: {e}", dest.display()))?;
    }
    if let Some(age) = file_age(&src) {
        let _ = set_file_age(&dest, age);
    }
    // EFS.MarkFile(src, true): the import file is renamed out of the way, never deleted
    let ext = src.extension().map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    let marked: PathBuf = src.with_extension(format!("~{ext}"));
    std::fs::rename(&src, &marked).map_err(|e| format!("{} could not be renamed: {e}", src.display()))?;
    Ok(name)
}

/// Every source picture of the library against its description and game texture
/// (`SynchronizeTextures(true, true, false)`): a description missing or of another age is
/// written, a game texture missing or of another age is built.
pub fn synchronize_all(
    dirs: &EditorDirs,
    cancel: &gz_task::Cancel,
    mut report: impl FnMut(usize, usize, &str),
) -> Result<(usize, Vec<String>), String> {
    let sources = crate::paths::list_files(&dirs.textures, &["tga"], false, true);
    let age = now();
    let mut updated = 0;
    let mut failed = Vec::new();
    for (i, (name, path)) in sources.iter().enumerate() {
        cancel.check().map_err(|_| "Stopped.".to_owned())?;
        report(i, sources.len(), name);
        let base_age = file_age(path);
        let thm_age = file_age(&dirs.textures.join(rel(name, "thm")));
        let dds_age = file_age(&dirs.game_textures.join(rel(name, "dds")));
        let thm = thm_age.is_none() || thm_age != base_age;
        let game = thm || dds_age.is_none() || dds_age != base_age;
        if !thm && !game {
            continue;
        }
        match sync_texture(dirs, name, None, thm, game, age) {
            Ok(_) => updated += 1,
            Err(e) => failed.push(e),
        }
    }
    Ok((updated, failed))
}
