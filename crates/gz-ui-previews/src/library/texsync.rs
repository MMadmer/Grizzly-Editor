//! What the Texture Library writes: a description (`.thm`) beside its source
//! picture in `rawdata\textures`, the game texture (`.dds`) in `gamedata\textures`, and - for
//! New Textures - the source copied over from the import folder. Blocking, for jobs.

use std::path::{Path, PathBuf};

use gz_fmt_tex::{
    Image, TextureParams, TextureThm, TextureType, TgaDepth, is_valid_size, load_image, make_game_texture,
    read_texture_thm, write_texture_thm, write_tga,
};

use crate::paths::{file_age, texture_folder_name};
use crate::writer::LibraryWriter;

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
    /// The bump map's gloss mask is under 10 percent.
    pub low_gloss: bool,
}

/// Writes a texture's description and its game texture: `params` are
/// the edited ones, `None` keeps those on disk (or makes the defaults of a new picture).
pub fn sync_texture(
    dirs: &LibraryWriter,
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
        dirs.write(&thm_path, &bytes)?;
        out.thm = true;
    }
    let dds_path = dirs.game_textures.join(rel(name, "dds"));
    if write_game {
        if !is_valid_size(image.width, image.height) {
            return Err(format!(
                "The game texture '{name}' could not be made: {}x{} is not a size the game takes.",
                image.width, image.height
            ));
        }
        // a bump map may name a normal map of its own, a 32-bit normal map of the same size
        let normal = if params.ty == TextureType::BUMP_MAP && !params.ext_normal_map_name.is_empty() {
            let nm = params.ext_normal_map_name.to_string();
            let path = dirs.game_textures.join(rel(&nm, "dds"));
            let (img, _) = read_picture(&path)
                .map_err(|_| format!("The normal map '{nm}' of '{name}' could not be read."))?;
            if img.width != image.width || img.height != image.height {
                return Err(format!(
                    "The normal map '{nm}' of '{name}' must be {}x{} like the picture.",
                    image.width, image.height
                ));
            }
            Some(img)
        } else {
            None
        };
        let made = make_game_texture(&image, &params, normal.as_ref())
            .map_err(|e| format!("The game texture '{name}' could not be made: {e}"))?;
        dirs.write(&dds_path, &made.dds)?;
        let bump = dirs.game_textures.join(format!("{}#.dds", name.replace('/', "\\")));
        match &made.bump_error_dds {
            Some(b) => dirs.write(&bump, b)?,
            None => dirs.remove_file(&bump)?,
        }
        out.game = true;
        out.low_gloss = made.low_gloss;
    }
    if out.thm || out.game {
        for p in [&tga, &thm_path, &dds_path] {
            if p.is_file() {
                // a file that keeps its old age is synchronised again next time: no failure
                let _ = dirs.set_age(p, age);
            }
        }
    }
    Ok(out)
}

/// A picture of the import folder taken into the library: copied as a `.tga` under its folder
/// name (`wpn_ak74` into `wpn\`), its age kept. The import file is renamed out of the way to
/// `.~<ext>` - it is the author's and is never deleted - unless the import itself staged it
/// there (`staged`): such a copy is removed, the picture is in the library now. Returns the
/// library name.
pub fn import_picture(dirs: &LibraryWriter, file: &str, staged: bool) -> Result<String, String> {
    let src = dirs.import.join(file);
    let stem = crate::paths::strip_ext(file);
    let name = texture_folder_name(stem);
    let dest = dirs.textures.join(rel(&name, "tga"));
    if let Some(dir) = dest.parent() {
        dirs.create_dir_all(dir)?;
    }
    let is_tga = file.to_lowercase().ends_with(".tga");
    if is_tga {
        dirs.copy(&src, &dest)?;
    } else {
        let (image, alpha) = read_picture(&src)?;
        let tga = write_tga(&image, if alpha { TgaDepth::Bgra32 } else { TgaDepth::Bgr24 })
            .map_err(|e| format!("{file} could not be converted: {e}"))?;
        dirs.write(&dest, &tga)?;
    }
    if let Some(age) = file_age(&src) {
        let _ = dirs.set_age(&dest, age);
    }
    if staged {
        return dirs.remove_file(&src).map(|()| name);
    }
    let ext = src.extension().map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    let marked: PathBuf = src.with_extension(format!("~{ext}"));
    dirs.rename(&src, &marked)?;
    Ok(name)
}

/// What a synchronisation of the textures did.
#[derive(Debug, Default)]
pub struct TexSyncReport {
    /// The textures written again, by name.
    pub updated: Vec<String>,
    /// How many were current already and left as they were.
    pub up_to_date: usize,
    /// The sentences of the ones that could not be made.
    pub failed: Vec<String>,
}

/// Every source picture of the library against its description and game texture: a description
/// missing or of another age than its picture is written, a game texture missing or of another
/// age is built. What is current is left as it is.
pub fn synchronize_all(
    dirs: &LibraryWriter,
    cancel: &gz_task::Cancel,
    mut report: impl FnMut(usize, usize, &str),
) -> Result<TexSyncReport, String> {
    let sources = crate::paths::list_files(&dirs.textures, &["tga"], false, true);
    let age = now();
    let mut out = TexSyncReport::default();
    for (i, (name, path)) in sources.iter().enumerate() {
        cancel.check().map_err(|_| "Stopped.".to_owned())?;
        report(i, sources.len(), name);
        let base_age = file_age(path);
        let thm_age = file_age(&dirs.textures.join(rel(name, "thm")));
        let dds_age = file_age(&dirs.game_textures.join(rel(name, "dds")));
        let thm = thm_age.is_none() || thm_age != base_age;
        let game = thm || dds_age.is_none() || dds_age != base_age;
        if !thm && !game {
            out.up_to_date += 1;
            continue;
        }
        match sync_texture(dirs, name, None, thm, game, age) {
            Ok(_) => out.updated.push(name.clone()),
            Err(e) => out.failed.push(e),
        }
    }
    Ok(out)
}
