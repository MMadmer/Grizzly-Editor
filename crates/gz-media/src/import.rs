//! `asset.import`: files of common formats brought into the project, each with its asset.
//!
//! A source is copied into the target folder (a `.gltf` with its buffers and pictures, an
//! `.obj` with its material libraries and their pictures, keeping where they lie beside it; the
//! pictures a `.glb` holds written out as `.png`), its asset is written beside it, and every
//! picture a model's surface wears gets a texture asset of its own, named in the model's
//! `surfaces`. What would be written is worked out first ([`plan`]); nothing is written when
//! the plan holds a file that is there and the caller did not say `overwrite`.
//!
//! A file of the game's own formats brings what it says of itself into its asset: a texture's
//! `.thm` beside it (the description in words - over a game texture, published as it is, only
//! what the game reads; the `.thm` stays unless the caller has it removed, [`Plan::removes`]),
//! a sound's parameters from its `.ogg` (or a `.thm` beside a `.wav`), and a compiled model of a
//! `gamedata\meshes` tree is a dynamic model, the one the game loads by its name. What an asset
//! would say otherwise than the file - read back from the very text written - gets no asset:
//! the file stays as it is. A file of a game data tree keeps the name the game knows it by: the
//! one of where it lands in the module's own game data, else the one of where it lay. A file of
//! the project stays where it lies and gets its asset beside it, unless it is brought into
//! another folder; a `.thm` alone describes a texture of the game. A file of the game's formats
//! in the module's `gamedata` outside its kind's folder (a level's lightmap) is read by the game
//! where it lies and gets no asset. A file that has its asset already is left as it is (`kept`).

use std::path::{Path, PathBuf};

use gz_base::RelPath;
use gz_fmt_mesh::PictureData;
use gz_luadata::{Table, Value, write_asset};
use gz_project::asset_kind::MediaKind;

use crate::files;
use crate::resolve::embedded_stem;

/// What a planned file is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Copy(PathBuf),
    Bytes(Vec<u8>),
}

/// The files an import writes, and what it says about them.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub writes: Vec<(RelPath, Content)>,
    /// The assets it writes, project-relative.
    pub assets: Vec<String>,
    /// The files it copies or writes out, project-relative.
    pub copied: Vec<String>,
    /// One sentence per file it leaves out, and why.
    pub problems: Vec<String>,
    /// The files of the project that have their asset already, left as they are.
    pub kept: Vec<String>,
    /// The `.thm` files of the project its assets now say all the game reads of: removed once the
    /// assets are written when the caller asks for it (`remove_descriptions`), kept otherwise.
    pub removes: Vec<RelPath>,
}

impl Plan {
    fn remove(&mut self, rel: RelPath) {
        if !self.removes.iter().any(|r| r.key() == rel.key()) {
            self.removes.push(rel);
        }
    }

    fn add(&mut self, rel: RelPath, content: Content, asset: bool) {
        if self.writes.iter().any(|(r, _)| r.key() == rel.key()) {
            return;
        }
        if asset {
            self.assets.push(rel.to_string());
        } else {
            self.copied.push(rel.to_string());
        }
        self.writes.push((rel, content));
    }
}

/// Writes what a plan writes and removes the descriptions it says all of, on the IO pool: each
/// file atomically, said to the project's watcher as the editor's own write, the file tree
/// brought up to date after. The sentence of the first failure, with what was written before
/// it kept.
pub async fn write(
    ed: &std::sync::Arc<gz_editor::Editor>,
    project: &std::sync::Arc<gz_project::Project>,
    root: &Path,
    writes: Vec<(RelPath, Content)>,
    removes: Vec<RelPath>,
) -> Result<(), String> {
    let (r, watch) = (root.to_path_buf(), project.clone());
    let written = ed
        .exec
        .io(move || -> Result<Vec<PathBuf>, String> {
            let mut done = Vec::new();
            for (rel, content) in writes {
                let abs = rel.to_fs(&r);
                let bytes = match content {
                    Content::Bytes(b) => b,
                    Content::Copy(src) => std::fs::read(&src)
                        .map_err(|e| format!("{}: {}", src.display(), gz_base::io_reason(&e)))?,
                };
                if let Some(dir) = abs.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
                }
                gz_editor::fsx::write_atomic(&abs, &bytes)
                    .map_err(|e| format!("{rel}: {}", gz_base::io_reason(&e)))?;
                watch.tree.changes().note_written(&rel, &bytes);
                done.push(abs);
            }
            // the descriptions the assets now say all of, once every asset is written
            for rel in removes {
                let abs = rel.to_fs(&r);
                match std::fs::remove_file(&abs) {
                    Ok(()) => done.push(abs),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{rel}: {}", gz_base::io_reason(&e))),
                }
            }
            Ok(done)
        })
        .await?;
    for abs in &written {
        project.tree.mark_fs(abs);
    }
    project.tree.refresh().await;
    Ok(())
}

/// The folder each kind goes into when the import names none.
#[must_use]
pub fn default_folder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Model => "models",
        MediaKind::Texture => "textures",
        MediaKind::Sound => "sounds",
    }
}

fn leaf(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn join(dir: &RelPath, tail: &str) -> Option<RelPath> {
    if dir.as_str().is_empty() { RelPath::new(tail).ok() } else { dir.join(tail).ok() }
}

/// The text of a new asset that names its source.
fn asset_text(kind: MediaKind, source: &str, surfaces: Vec<Value>) -> Vec<u8> {
    let mut top = Table::new();
    top.put(kind.header(), kind.version());
    top.put("source", source);
    if !surfaces.is_empty() {
        top.put("surfaces", Value::from(surfaces));
    }
    write_asset("", &top).into_bytes()
}

/// A picture as `.png`: a PNG as it is, anything else the editor reads decoded and written
/// again.
fn as_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(b"\x89PNG") {
        return Ok(bytes.to_vec());
    }
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// The `.thm` beside a picture or a sound (`<stem>.thm`), when there is one.
fn thm_beside(src: &Path) -> Option<PathBuf> {
    let p = src.with_extension("thm");
    p.is_file().then_some(p)
}

/// A number as the shortest text that reads back to its `f32`: what a description holds.
fn short(v: f32) -> f64 {
    format!("{v}").parse().unwrap_or(f64::from(v))
}

/// A description with what only the making of a picture reads set aside: what a game texture,
/// published as it is, says to whoever reads it.
fn as_published(p: &gz_fmt_tex::TextureParams) -> gz_fmt_tex::TextureParams {
    use gz_fmt_tex::TextureFlags as F;
    let d = gz_fmt_tex::TextureParams::default();
    let mut p = p.clone();
    p.fmt = d.fmt;
    p.mip_filter = d.mip_filter;
    p.border_color = d.border_color;
    p.fade_color = d.fade_color;
    p.fade_amount = d.fade_amount;
    p.fade_delay = d.fade_delay;
    p.bump_virtual_height = d.bump_virtual_height;
    p.ext_normal_map_name = d.ext_normal_map_name;
    p.flags.remove(
        F::GENERATE_MIP_MAPS
            | F::BINARY_ALPHA
            | F::ALPHA_BORDER
            | F::COLOR_BORDER
            | F::FADE_TO_COLOR
            | F::FADE_TO_ALPHA
            | F::DITHER_COLOR
            | F::DITHER_EACH_MIP_LEVEL,
    );
    p
}

/// The asset text of a texture made from its `.thm`: every key the description says - over a
/// game texture published as it is (`as_is`), only those something reads of it, the making keys
/// left out - and what the game reads of a texture said outright, its lighting model, its bump
/// map and its detail texture (`false` for none), so the description replaces whatever the
/// game's own texture of that name had. The text is read back and must say what the `.thm`
/// says; `None` when it cannot: the `.thm` stays the description.
fn texture_from_thm(params: &gz_fmt_tex::TextureParams, name: &str, as_is: bool) -> Option<Table> {
    use crate::texture::TextureAsset;
    let alpha = params.has_alpha();
    let asset = TextureAsset::from_params(params, name, alpha);
    let mut top = asset.to_table();
    if as_is {
        top.fields.retain(|(k, _)| !crate::texture::MAKING_KEYS.contains(&k.as_str()));
        if let Some(Value::Table(b)) = top.get_mut("bump") {
            b.fields.retain(|(k, _)| k.as_str() != "depth");
        }
    }
    // what the game reads, said whatever it is
    if top.get("material").is_none() {
        let word = crate::fields::word_of(crate::fields::MATERIALS, params.material.0).unwrap_or("blinn");
        top.put("material", word);
    }
    // a bump map of no name (`texture = ""`) and a detail texture of no name or laid on nothing
    // are what the game reads them as: none, the way the `.thm` says it
    if asset.bump.is_none() {
        top.put("bump", false);
    }
    if asset.detail.is_none() {
        top.put("detail", false);
    }
    // what is written is what is read back
    let value = gz_luadata::parse(&write_asset("", &top)).ok()?;
    let (again, issues) = TextureAsset::from_value(&value);
    if !issues.is_empty() {
        return None;
    }
    let back = again.to_params(name, (params.width.max(0) as u32, params.height.max(0) as u32), alpha);
    // a bump map's name means something only while the description uses a bump map: one left
    // behind by a tool under no bump is read by nobody, the game's renderer or the SDK
    let said = |p: &gz_fmt_tex::TextureParams| {
        let mut p = if as_is { as_published(p) } else { p.clone() };
        if !matches!(p.bump_mode, gz_fmt_tex::BumpMode::USE | gz_fmt_tex::BumpMode::USE_PARALLAX) {
            p.bump_name = Default::default();
        }
        p
    };
    (said(&back) == said(params)).then_some(top)
}

/// Whether a `.thm` says to the game what a texture asset does not (`M118`): what the game
/// reads of both differs, and the `.thm` says something at all. `as_is`: the picture is a game
/// texture published as it is.
#[must_use]
pub(crate) fn thm_says_other(
    thm: &gz_fmt_tex::TextureParams,
    asset: &crate::texture::TextureAsset,
    name: &str,
    as_is: bool,
) -> bool {
    if says_nothing_to_the_game(thm) {
        return false;
    }
    let size = (thm.width.max(0) as u32, thm.height.max(0) as u32);
    let ours = asset.to_params(name, size, thm.has_alpha());
    let said = |p: &gz_fmt_tex::TextureParams| {
        let mut p = if as_is { as_published(p) } else { p.clone() };
        if !matches!(p.bump_mode, gz_fmt_tex::BumpMode::USE | gz_fmt_tex::BumpMode::USE_PARALLAX) {
            p.bump_name = Default::default();
        }
        // the source picture's size is the picture's, never a word of either
        p.width = 0;
        p.height = 0;
        p
    };
    said(&ours) != said(thm)
}

/// Whether a texture description says nothing the game reads of a texture beyond what it gives
/// one nobody describes: no bump map, no detail texture, the lighting model `blinn` with no
/// weight (or a type the game describes nothing of). The `.thm` a tool leaves beside a bump map's
/// `#` companion is such a one: it goes with the texture's asset, which publishes the companion.
fn says_nothing_to_the_game(p: &gz_fmt_tex::TextureParams) -> bool {
    use gz_fmt_tex::{BumpMode, TextureFlags, TextureType};
    let described = matches!(p.ty, TextureType::IMAGE | TextureType::TERRAIN | TextureType::NORMAL_MAP);
    if !described {
        return true;
    }
    let lit = f32::from(u8::try_from(p.material.0).unwrap_or(u8::MAX)) + p.material_weight;
    let bump = matches!(p.bump_mode, BumpMode::USE | BumpMode::USE_PARALLAX);
    let detail = !p.detail_name.is_empty()
        && p.flags.intersects(TextureFlags::DIFFUSE_DETAIL | TextureFlags::BUMP_DETAIL);
    lit == 1.0 && !bump && !detail
}

/// The keys a sound asset says of a game sound: the parameters in its `.ogg`, or a `.thm`
/// beside it - read back from the text and the same, or an error: the sound stays as it is.
fn sound_keys(src: &Path) -> Result<Table, String> {
    let thm = if src.extension().is_some_and(|e| e.eq_ignore_ascii_case("ogg")) {
        std::fs::read(src).ok().and_then(|b| gz_fmt_sound::read_ogg_params(&b).ok().flatten()).map(|o| {
            gz_fmt_sound::SoundThm {
                min_dist: o.min_dist,
                max_dist: o.max_dist,
                max_ai_dist: o.max_ai_dist,
                base_volume: o.base_volume,
                game_type: o.game_type,
                ..gz_fmt_sound::SoundThm::default()
            }
        })
    } else {
        thm_beside(src)
            .and_then(|t| std::fs::read(t).ok())
            .and_then(|b| gz_fmt_sound::read_sound_thm(&b).ok())
    };
    let Some(thm) = thm else { return Ok(Table::new()) };
    let mut a = crate::sound::SoundAsset::from_thm(&thm);
    for v in [&mut a.min_distance, &mut a.max_distance, &mut a.volume, &mut a.ai_distance, &mut a.quality] {
        *v = v.map(|x| short(x as f32));
    }
    let mut t = a.to_table();
    // what is written is what is read back
    let again =
        gz_luadata::parse(&write_asset("", &t)).ok().map(|v| crate::sound::SoundAsset::from_value(&v));
    if !again.is_some_and(|(b, issues)| issues.is_empty() && b.to_thm() == thm) {
        return Err("its parameters say what a sound asset cannot say the same way".to_owned());
    }
    t.fields.retain(|(k, _)| k.as_str() != "snd");
    Ok(t)
}

/// The text of an asset: its header, its source (left out when the asset finds it by its own
/// name), its name when it is not the default, and the keys the file says of itself.
fn asset_of(
    kind: MediaKind,
    source: Option<&str>,
    name: Option<&str>,
    keys: Table,
    surfaces: Vec<Value>,
) -> Vec<u8> {
    let mut top = Table::new();
    top.put(kind.header(), kind.version());
    if let Some(s) = source {
        top.put("source", s);
    }
    if let Some(n) = name {
        top.put("name", n);
    }
    for (k, v) in keys.fields {
        if !matches!(k.as_str(), "tex" | "snd" | "mesh" | "source" | "name") {
            top.put(k.as_str(), v);
        }
    }
    if !surfaces.is_empty() {
        top.put("surfaces", Value::from(surfaces));
    }
    write_asset("", &top).into_bytes()
}

/// What importing `from` into `to` (left out: each kind's folder) writes; `root` is the
/// project's folder. `module` names the textures a model's surfaces wear. Blocking.
#[must_use]
pub fn plan(from: &[PathBuf], to: Option<&RelPath>, module: &str, root: &Path) -> Plan {
    let mut plan = Plan::default();
    let mut seen: Vec<String> = Vec::new();
    // a `#` companion whose description says something is a texture of its own, brought in too
    let mut work: Vec<PathBuf> = from.to_vec();
    let mut at = 0;
    while at < work.len() {
        let src = work[at].clone();
        at += 1;
        let src = &src;
        let name = leaf(src);
        let lower = name.to_ascii_lowercase();
        // a bump pair's `#` companion given with its texture goes with that texture
        if at <= from.len()
            && let Some(stem) = lower.strip_suffix("#.dds")
        {
            let texture = src.with_file_name(format!("{stem}.dds")).to_string_lossy().to_lowercase();
            if from.iter().any(|f| f.to_string_lossy().to_lowercase() == texture) {
                continue;
            }
        }
        // a description alone: of the picture beside it, or of a texture of the game
        let (src, game_description) = if lower.ends_with(".thm") {
            match files::source_extensions(MediaKind::Texture)
                .iter()
                .map(|e| src.with_extension(e))
                .find(|p| p.is_file())
            {
                Some(picture) => (picture, false),
                None => (src.clone(), true),
            }
        } else {
            (src.clone(), false)
        };
        let key = src.to_string_lossy().to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let name = leaf(&src);
        let kind = if game_description { Some(MediaKind::Texture) } else { files::kind_of_source(&name) };
        let Some(kind) = kind else {
            plan.problems.push(format!(
                "{name}: not a model (.glb, .gltf, .obj, .object, .ogf), a picture (.png, .jpg, .tga, .bmp, .dds), a texture description (.thm) or a sound (.wav, .ogg)"
            ));
            continue;
        };
        if !src.is_file() {
            plan.problems.push(format!("{}: there is no such file", src.display()));
            continue;
        }
        let path = src.to_string_lossy().into_owned();
        // where it goes: a file of the project stays where it lies, unless it is brought into
        // another folder - then it is copied there like any other
        let own = files::inside(root, &src)
            .then(|| files::rel_of(root, &src))
            .flatten()
            .filter(|rel| to.is_none_or(|t| t.key() == rel.parent().unwrap_or_else(RelPath::root).key()));
        let (target, folder) = match &own {
            Some(rel) => (rel.clone(), rel.parent().unwrap_or_else(RelPath::root)),
            None => {
                let folder = match to {
                    Some(t) => t.clone(),
                    None => RelPath::new(default_folder(kind)).unwrap_or_else(|_| RelPath::root()),
                };
                let Some(target) = join(&folder, &name) else {
                    plan.problems.push(format!("{name}: the name cannot be a file of the project"));
                    continue;
                };
                (target, folder)
            }
        };
        let Some(asset) = join(&folder, &format!("{}{}", stem(&src), kind.extension())) else { continue };
        // the name the game knows it by: of where it lands in the module's own game data, else
        // of where it lay in another tree of the game's data
        let game_name =
            files::game_name_of(kind, target.as_str()).or_else(|| files::game_name_of(kind, &path));
        let as_is = game_description
            || match kind {
                MediaKind::Texture => files::is_game_texture_file(&src),
                MediaKind::Sound => lower_ext(&src) == "ogg",
                MediaKind::Model => lower_ext(&src) == "ogf",
            };
        // a file of the game's formats in the module's game data but outside its kind's folder
        // is read by the game where it lies (a level's lightmap): no asset names it otherwise
        let in_game_data =
            target.as_str().split('/').next().is_some_and(|t| t.eq_ignore_ascii_case("gamedata"));
        if as_is && in_game_data && files::game_name_of(kind, target.as_str()).is_none() {
            let folder = files::game_folder(kind);
            if own.is_none() {
                plan.add(target, Content::Copy(src.clone()), false);
                plan.problems.push(format!(
                    "{name}: the game reads it where it lies, outside gamedata\\{folder}; it is copied in as a file of its own"
                ));
            } else {
                plan.problems.push(format!(
                    "{name}: the game reads it where it lies, outside gamedata\\{folder}; it stays a file of its own"
                ));
            }
            continue;
        }
        if own.is_some() && asset.to_fs(root).is_file() {
            if kind == MediaKind::Texture && !game_description {
                companion_description(&mut plan, root, &src, &mut work);
            }
            plan.kept.push(asset.to_string());
            continue;
        }
        // a game texture the game cannot load stays a file of its own: no asset says it is one
        if kind == MediaKind::Texture && !game_description && files::is_game_texture_file(&src) {
            let unread = files::dds_head(&src)
                .map_err(|e| gz_base::io_reason(&e))
                .and_then(|b| gz_fmt_tex::read_dds_info(&b).map(|_| ()).map_err(|e| e.to_string()))
                .err();
            let why = match (game_name.as_deref().filter(|g| !files::texture_name_reached(g)), unread) {
                (Some(g), _) => Some(format!(
                    "the game never asks for a texture named '{g}' (its loader drops a `.dds` from a name, and no name ends in a space)"
                )),
                (None, Some(e)) => Some(format!("{e} - the game cannot load it as a texture")),
                (None, None) => None,
            };
            if let Some(why) = why {
                if own.is_none() {
                    plan.add(target, Content::Copy(src.clone()), false);
                    plan.problems.push(format!("{name}: {why}; it is copied in as a file of its own"));
                } else {
                    plan.problems.push(format!("{name}: {why}; it stays a file of its own"));
                }
                continue;
            }
        }
        // the name the game knows it by: the default of an asset in the module's own game data,
        // said outright for a file brought in from another tree of the game's data
        let default = files::published_name(None, asset.as_str(), module);
        let named = game_name.clone().filter(|g| crate::read::name_key(g) != crate::read::name_key(&default));
        let published = named.clone().unwrap_or(default);
        let mut keys = Table::new();
        let mut surfaces = Vec::new();
        let mut thm_used = None;
        match kind {
            MediaKind::Texture => {
                let thm = if game_description { Some(src.clone()) } else { thm_beside(&src) };
                if let Some(thm) = thm {
                    match std::fs::read(&thm)
                        .map_err(|e| gz_base::io_reason(&e))
                        .and_then(|b| gz_fmt_tex::read_texture_thm(&b).map_err(|e| e.to_string()))
                    {
                        Ok(read) => match texture_from_thm(&read.params, &published, as_is) {
                            Some(t) => {
                                keys = t;
                                thm_used = Some(thm);
                            }
                            None if game_description => {
                                plan.problems.push(format!(
                                    "{name}: the texture description says what an asset cannot say the same way; it stays as it is"
                                ));
                                continue;
                            }
                            None => {
                                // the picture and its description stay what they are, shipped
                                // as they are
                                if own.is_none() {
                                    plan.add(target, Content::Copy(src.clone()), false);
                                    if let Some(t) = join(&folder, &leaf(&thm)) {
                                        plan.add(t, Content::Copy(thm.clone()), false);
                                    }
                                }
                                plan.problems.push(format!(
                                    "{}: the texture description says what an asset cannot say the same way; the picture and it stay files of their own",
                                    leaf(&thm)
                                ));
                                continue;
                            }
                        },
                        Err(why) => {
                            plan.problems.push(format!("{}: {why}", leaf(&thm)));
                            if game_description {
                                continue;
                            }
                        }
                    }
                }
            }
            MediaKind::Sound => match sound_keys(&src) {
                Ok(k) => keys = k,
                Err(why) => {
                    if own.is_none() {
                        plan.add(target, Content::Copy(src.clone()), false);
                        plan.problems.push(format!("{name}: {why}; it is copied in as a file of its own"));
                    } else {
                        plan.problems.push(format!("{name}: {why}; it stays a file of its own"));
                    }
                    continue;
                }
            },
            MediaKind::Model => {
                if lower_ext(&src) == "ogf" {
                    // a compiled model names its textures by their game names
                    if game_name.is_some() {
                        keys.put("kind", "dynamic");
                    }
                } else {
                    surfaces = model_extras(&mut plan, root, &src, &folder, module);
                }
            }
        }
        if game_description {
            // a description of the game's texture: its picture is the game's
            let Some(g) = game_name.clone() else {
                plan.problems.push(format!(
                    "{name}: a texture description alone names a texture of the game by where it lies under a gamedata\\textures folder"
                ));
                continue;
            };
            let Some(asset) = join(&folder, &format!("{}{}", stem(&src), kind.extension())) else { continue };
            let source = format!("{}{}", files::GAME_PREFIX, g.replace('\\', "/"));
            plan.add(asset, Content::Bytes(asset_of(kind, Some(&source), None, keys, Vec::new())), true);
        } else {
            let landed = target.to_fs(root);
            if own.is_none() {
                plan.add(target, Content::Copy(src.clone()), false);
                if let Some(c) = files::hash_companion(&src)
                    && let Some(t) = join(&folder, &leaf(&c))
                {
                    plan.add(t, Content::Copy(c), false);
                }
            }
            // a file of the game's formats found by the asset's own name - no other file of the
            // kind's formats by that name beside it - is named by nothing: a rename takes it along.
            // A source of several files (a glTF's buffers, an OBJ's materials) stays named: they
            // name each other by their names
            let by_name = as_is
                && !files::source_extensions(kind).iter().map(|e| asset.to_fs(root).with_extension(e)).any(
                    |p| p.is_file() && !p.to_string_lossy().eq_ignore_ascii_case(&landed.to_string_lossy()),
                )
                && stem(&landed).eq_ignore_ascii_case(&stem(&asset.to_fs(root)));
            let source = if by_name { None } else { Some(name.as_str()) };
            plan.add(asset, Content::Bytes(asset_of(kind, source, named.as_deref(), keys, surfaces)), true);
        }
        // the descriptions the assets now say: the project's own, which the caller may have removed
        if own.is_some() {
            if let Some(thm) = thm_used.and_then(|t| files::rel_of(root, &t)) {
                plan.remove(thm);
            }
            if kind == MediaKind::Texture && !game_description {
                companion_description(&mut plan, root, &src, &mut work);
            }
        }
    }
    plan
}

/// The description a tool left beside the `#` companion a texture publishes: it goes when it
/// says nothing to the game; when it does, the companion is a texture of its own and is brought
/// in with its asset (`work`), which the texture then leaves it to.
fn companion_description(plan: &mut Plan, root: &Path, src: &Path, work: &mut Vec<PathBuf>) {
    let Some(companion) = files::hash_companion(src) else { return };
    let Some(thm) = thm_beside(&companion) else { return };
    let Some(rel) = files::rel_of(root, &thm) else { return };
    let said = std::fs::read(&thm).ok().and_then(|b| gz_fmt_tex::read_texture_thm(&b).ok());
    match said {
        Some(t) if says_nothing_to_the_game(&t.params) => plan.remove(rel),
        _ => {
            if !work.iter().any(|w| w.as_path() == companion.as_path()) {
                work.push(companion);
            }
        }
    }
}

fn lower_ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// A sound brought in as a sound asset ([`sound_asset`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundImport {
    /// The sound asset, project-relative (`/`).
    pub asset: String,
    /// The name the game knows it by.
    pub name: String,
    /// The asset was written now; false: it was there.
    pub written: bool,
}

/// Music brought in as a sound asset, the way `asset.import` brings a sound: a file of the
/// project stays where it lies, one from outside is copied into `sounds\` (the same bytes there
/// already are that file), and the asset is written beside it unless one is there over it. The
/// name is the module's `<module id>\<file name>`, which no sound of the game or of another
/// module has; one that `taken` (the names the project's sound assets publish) or another file's
/// asset holds gets `_2`, `_3`, ... A sound asset of the project given is that sound. `guard`
/// passes every write first. Blocking.
pub fn sound_asset(
    root: &Path,
    from: &Path,
    module: &str,
    taken: &dyn Fn(&str) -> bool,
    guard: Option<&gz_editor::fsx::WriteGuard>,
) -> Result<SoundImport, String> {
    let name = leaf(from);
    let write = |abs: &Path, bytes: &[u8]| -> Result<(), String> {
        if let Some(g) = guard {
            g.check(abs).map_err(|e| e.0)?;
        }
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
        }
        gz_editor::fsx::write_atomic(abs, bytes).map_err(|e| format!("{name}: {}", gz_base::io_reason(&e)))
    };
    let rel_of = |abs: &Path| {
        files::rel_of(root, abs)
            .map(|r| r.as_str().to_owned())
            .ok_or_else(|| format!("{name}: the file cannot be a file of the project"))
    };
    if MediaKind::of_file(&name) == Some(MediaKind::Sound) {
        if !files::inside(root, from) {
            return Err(format!(
                "{name}: a sound asset of another folder - bring in its sound file (.wav, .ogg) instead"
            ));
        }
        let text =
            std::fs::read_to_string(from).map_err(|e| format!("{name}: {}", gz_base::io_reason(&e)))?;
        let asset = rel_of(from)?;
        let value = gz_luadata::parse(&text).map_err(|e| format!("{name}: {}", e.message))?;
        let (a, _) = crate::sound::SoundAsset::from_value(&value);
        return Ok(SoundImport {
            name: files::published_name(a.name.as_deref(), &asset, module),
            asset,
            written: false,
        });
    }
    if files::kind_of_source(&name) != Some(MediaKind::Sound) {
        return Err(format!("{name}: not a sound (.wav, .ogg)"));
    }
    if !from.is_file() {
        return Err(format!("{}: there is no such file", from.display()));
    }
    // where the sound lies in the project
    let source = if files::inside(root, from) {
        from.to_path_buf()
    } else {
        let bytes = std::fs::read(from).map_err(|e| format!("{name}: {}", gz_base::io_reason(&e)))?;
        let folder = root.join(default_folder(MediaKind::Sound));
        let ext = from.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let base = stem(from);
        let mut n = 1u32;
        loop {
            let candidate =
                if n == 1 { folder.join(&name) } else { folder.join(format!("{base}_{n}.{ext}")) };
            if !candidate.exists() {
                write(&candidate, &bytes)?;
                break candidate;
            }
            // the same file there already is the one to use
            if std::fs::read(&candidate).is_ok_and(|there| there == bytes) {
                break candidate;
            }
            n += 1;
            if n > 10_000 {
                return Err(format!("{name}: the file cannot be copied into the project"));
            }
        }
    };
    let file = leaf(&source);
    let base = stem(&source);
    let dir = source.parent().unwrap_or(root).to_path_buf();
    let mut n = 1u32;
    loop {
        let s = if n == 1 { base.clone() } else { format!("{base}_{n}") };
        let abs = dir.join(format!("{s}{}", MediaKind::Sound.extension()));
        let asset = rel_of(&abs)?;
        if abs.is_file() {
            // an asset there already over this very sound is the sound
            let key = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
            if let Ok((a, src, _)) = crate::resolve::sound(root, &asset)
                && key(&src) == key(&source)
            {
                let name = files::published_name(a.name.as_deref(), &asset, module);
                return Ok(SoundImport { asset, name, written: false });
            }
        } else {
            let published = files::published_name(None, &asset, module);
            if !taken(&published) {
                write(&abs, &asset_text(MediaKind::Sound, &file, Vec::new()))?;
                return Ok(SoundImport { asset, name: published, written: true });
            }
        }
        n += 1;
        if n > 10_000 {
            return Err(format!("{name}: no name is free for its sound asset"));
        }
    }
}

/// A model's files beside it and the texture assets of its pictures; answers its `surfaces`. A
/// file that lies where it would go already (a model of the project imported in place) is not
/// copied, and a picture that has its texture asset keeps it.
fn model_extras(plan: &mut Plan, root: &Path, src: &Path, folder: &RelPath, module: &str) -> Vec<Value> {
    let dir = src.parent().unwrap_or(Path::new(""));
    let place = |p: &Path| -> Option<String> {
        let rel = p.strip_prefix(dir).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    };
    let there = |rel: &RelPath, p: &Path| {
        rel.to_fs(root).to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())
    };
    let deps = files::dependencies(src);
    for d in &deps {
        match place(d).and_then(|r| join(folder, &r)) {
            Some(rel) if there(&rel, d) => {}
            Some(rel) => plan.add(rel, Content::Copy(d.clone()), false),
            None => plan.problems.push(format!(
                "{}: {} lies outside its folder and is not copied",
                leaf(src),
                d.display()
            )),
        }
    }
    let is_object = src.extension().is_some_and(|e| e.eq_ignore_ascii_case("object"));
    if is_object {
        return Vec::new();
    }
    let source = match gz_fmt_mesh::read(src) {
        Ok(s) => s,
        Err(e) => {
            plan.problems.push(format!("{}: {e} - copied, its asset will say so", leaf(src)));
            return Vec::new();
        }
    };
    let model = stem(src);
    let mut surfaces = Vec::new();
    for m in &source.materials {
        let Some(pi) = m.picture else { continue };
        let Some(pic) = source.pictures.get(pi) else { continue };
        // where the picture lands in the project, and its file name there
        let landed: Option<(RelPath, String)> = match &pic.data {
            PictureData::File(p) => {
                if !p.is_file() {
                    plan.problems.push(format!("{}: the picture {} is not there", leaf(src), p.display()));
                    continue;
                }
                match place(p).and_then(|r| join(folder, &r)) {
                    Some(rel) => {
                        if !there(&rel, p) {
                            plan.add(rel.clone(), Content::Copy(p.clone()), false);
                        }
                        Some((rel, leaf(p)))
                    }
                    None => None,
                }
            }
            PictureData::Embedded(bytes) => match as_png(bytes) {
                Ok(png) => {
                    let file = format!("{}.png", embedded_stem(&model, &pic.name, pi));
                    join(folder, &file).map(|rel| {
                        plan.add(rel.clone(), Content::Bytes(png), false);
                        (rel, file)
                    })
                }
                Err(e) => {
                    plan.problems.push(format!(
                        "{}: the picture '{}' cannot be read: {e}",
                        leaf(src),
                        pic.name
                    ));
                    None
                }
            },
        };
        let Some((rel, file)) = landed else { continue };
        let pic_stem =
            Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let tex_rel = rel.parent().and_then(|p| join(&p, &format!("{pic_stem}.texasset")));
        if let Some(t) = tex_rel {
            if !t.to_fs(root).is_file() {
                plan.add(t.clone(), Content::Bytes(asset_text(MediaKind::Texture, &file, Vec::new())), true);
            }
            let mut row = Table::new();
            row.put("name", m.name.as_str());
            row.put("texture", files::published_name(None, t.as_str(), module));
            surfaces.push(Value::Table(row));
        }
    }
    surfaces
}
