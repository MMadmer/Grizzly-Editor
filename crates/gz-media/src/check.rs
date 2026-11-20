//! `asset.check` of a media asset: every problem at the key it is about.
//!
//! | Code | |
//! |---|---|
//! | `M101` | the source is not there (a file, or a model of the game) |
//! | `M102` | the source cannot be read (the reader's sentence) |
//! | `M103` | two assets of a kind publish one name (on both) |
//! | `M104` | a texture nobody has: no texture asset, no file of the project, not the game's (warning) |
//! | `M105` `M106` | an engine shader, a compiler shader the linked game lacks (warning) |
//! | `M107` | a game material the linked game lacks (a surface's; a bone's is a warning) |
//! | `M108` | a dynamic model without bones gets one (note) |
//! | `M109` | a value outside what its key takes |
//! | `M110` | `parts` together with `source` |
//! | `M111` | what the asset cannot take: a surface row the model lacks, a key that says how a picture is made over a game texture (warnings); a description of the game's texture with a name or a bump picture of its own |
//! | `M112` | a surface that wears no texture |
//! | `M113` | a source that lies outside the project |
//! | `M114` | a sound's `streamer` on a sound that is not copyrighted (warning) |
//! | `M115` | a sound's replacement that is copyrighted itself |
//! | `M116` | a sound's replacement nobody has |

use std::path::Path;
use std::sync::Arc;

use gz_asset::AssetFile;
use gz_base::{Problem, Severity};
use gz_editor::Editor;
use gz_luadata::Seg;
use gz_project::asset_kind::MediaKind;

use crate::index::Index;
use crate::model::ModelAsset;
use crate::read::{Issue, name_key};
use crate::resolve;
use crate::service::{GameLists, Media};
use crate::sound::SoundAsset;
use crate::texture::TextureAsset;

pub const TWICE: &str = "M103";
pub const NO_TEXTURE: &str = "M104";
pub const NO_SHADER: &str = "M105";
pub const NO_COMPILER_SHADER: &str = "M106";
pub const NO_MATERIAL: &str = "M107";
pub const BARE_SURFACE: &str = "M112";
pub const STREAMER_UNUSED: &str = "M114";
pub const STREAMER_COPYRIGHTED: &str = "M115";
pub const STREAMER_MISSING: &str = "M116";

fn problem(i: &Issue) -> Problem {
    Problem::new(i.code, i.severity, i.message.clone())
}

/// Whether anybody has a texture: a texture asset of the project, a file of the project, the
/// linked game. Blocking.
#[must_use]
pub fn texture_known(root: &Path, index: &Index, game: &GameLists, name: &str) -> bool {
    let key = name_key(name);
    if key.is_empty() || index.by_name(MediaKind::Texture, &key).is_some() || game.has_texture(&key) {
        return true;
    }
    let rel: Vec<&str> = key.split('\\').filter(|s| !s.is_empty()).collect();
    let under = |top: &str, dir: &str, ext: &str| {
        let mut p = root.join(top).join(dir);
        p.extend(&rel);
        p.set_extension(ext);
        p.is_file()
    };
    under("gamedata", "textures", "dds")
        || ["tga", "png", "dds"].iter().any(|e| under("rawdata", "textures", e))
        // without a game nothing says the texture is missing
        || !game.linked
}

/// The issues of a file's own values, and what only the project and the game can tell.
#[must_use]
pub fn issues(
    root: &Path,
    rel: &str,
    kind: MediaKind,
    value: &gz_luadata::Value,
    index: &Index,
    game: &GameLists,
    models: &resolve::GameModel,
) -> Vec<Issue> {
    let mut out = match kind {
        MediaKind::Model => ModelAsset::from_value(value).1,
        MediaKind::Texture => TextureAsset::from_value(value).1,
        MediaKind::Sound => SoundAsset::from_value(value).1,
    };
    // one name, two assets: on both
    if let Some(me) = index.at(rel).filter(|e| !e.excluded) {
        let others: Vec<String> = index
            .named(kind, &me.name)
            .iter()
            .filter(|e| e.path != me.path)
            .map(|e| e.path.to_string())
            .collect();
        if !others.is_empty() {
            let at = if value.get("name").is_some() { vec![Seg::key("name")] } else { Vec::new() };
            out.push(Issue::new(
                TWICE,
                Severity::Error,
                at,
                format!("another {} publishes the name '{}': {}", kind.name(), me.name, others.join(", ")),
            ));
        }
    }
    let warn = |code: &'static str, path: Vec<Seg>, message: String| {
        Issue::new(code, Severity::Warning, path, message)
    };
    match kind {
        MediaKind::Model => match resolve::model(root, rel, models) {
            Err(f) => out.push(f.issue),
            Ok(m) => {
                out.extend(m.issues.into_iter().filter(|i| i.code != crate::read::BAD_VALUE));
                let asset = &m.asset;
                // a row that names no surface of the model changes nothing: said, with the
                // names there are
                for row in &asset.surfaces {
                    if m.object.surfaces.iter().any(|s| s.name.eq_ignore_ascii_case(&row.name)) {
                        continue;
                    }
                    let mut known: Vec<&str> = m.object.surfaces.iter().map(|s| s.name.as_str()).collect();
                    let more = known.len().saturating_sub(12);
                    known.truncate(12);
                    out.push(Issue::new(
                        resolve::CANNOT,
                        Severity::Warning,
                        vec![Seg::key("surfaces"), Seg::item("name", &row.name)],
                        format!(
                            "the model has no surface '{}' (it has: {}{})",
                            row.name,
                            known.join(", "),
                            if more > 0 { format!(" and {more} more") } else { String::new() }
                        ),
                    ));
                }
                for s in &m.object.surfaces {
                    let row = asset.surfaces.iter().find(|r| r.name.eq_ignore_ascii_case(&s.name));
                    let at = |key: &str| match row {
                        Some(r) => vec![Seg::key("surfaces"), Seg::item("name", &r.name), Seg::key(key)],
                        None => vec![Seg::key("surfaces")],
                    };
                    // no level build and no model the game loads takes a surface without one
                    if s.texture.trim().is_empty() {
                        out.push(Issue::new(
                            BARE_SURFACE,
                            Severity::Error,
                            at("texture"),
                            format!(
                                "the surface '{}' wears no texture: name one in `surfaces` ({{ name = \"{}\", texture = ... }}) - a texture asset of the project or a texture of the game",
                                s.name, s.name
                            ),
                        ));
                    }
                    if !s.texture.is_empty() && !texture_known(root, index, game, &s.texture) {
                        out.push(warn(
                                NO_TEXTURE,
                                at("texture"),
                                format!("the surface '{}' wears the texture '{}', which nobody has: no texture asset, no file of the project, not the game's", s.name, s.texture),
                            ));
                    }
                    if !game.linked {
                        continue;
                    }
                    if !s.shader.is_empty() && !game.shaders.contains(&s.shader.to_lowercase()) {
                        out.push(warn(
                            NO_SHADER,
                            at("shader"),
                            format!("the linked game has no engine shader '{}'", s.shader),
                        ));
                    }
                    if !s.shader_xrlc.is_empty()
                        && !game.compiler_shaders.contains(&s.shader_xrlc.to_lowercase())
                    {
                        out.push(warn(
                            NO_COMPILER_SHADER,
                            at("compile"),
                            format!("the linked game has no compiler shader '{}'", s.shader_xrlc),
                        ));
                    }
                    // the level build refuses the surface, so this is no warning
                    if !s.game_mtl.is_empty() && !game.materials.contains(&s.game_mtl.to_lowercase()) {
                        out.push(Issue::new(
                            NO_MATERIAL,
                            Severity::Error,
                            at("material"),
                            format!(
                                "the linked game has no game material '{}' (its own are `default`, `materials\\wood`, ...: gamemtl.xr)",
                                s.game_mtl
                            ),
                        ));
                    }
                }
                if game.linked {
                    for b in &asset.bones {
                        let Some(mtl) = &b.material else { continue };
                        let key = mtl.to_lowercase();
                        let at = vec![Seg::key("bones"), Seg::item("name", &b.name), Seg::key("material")];
                        if !game.materials.contains(&key) {
                            out.push(warn(
                                NO_MATERIAL,
                                at,
                                format!("the linked game has no game material '{mtl}': the bone keeps the model's own"),
                            ));
                        } else if !game.dynamic_materials.contains(&key) {
                            // the game takes a bone's material from the description only when
                            // it is one a moving body is made of
                            out.push(warn(
                                NO_MATERIAL,
                                at,
                                format!(
                                    "'{mtl}' is no dynamic game material: a bone takes one a moving body is made of (`objects\\...`, `default_object`), and the game keeps the model's own"
                                ),
                            ));
                        }
                    }
                }
            }
        },
        MediaKind::Texture => {
            let name = index.at(rel).map(|e| e.name.clone()).unwrap_or_default();
            match texture_facts(root, rel, &name, game, &mut out) {
                Err(f) => out.push(f.issue),
                Ok(t) => {
                    let mut named: Vec<(Vec<Seg>, String)> = Vec::new();
                    if let Some(d) = t.asset.detail.as_ref().and_then(|d| d.texture.clone()) {
                        named.push((vec![Seg::key("detail"), Seg::key("texture")], d));
                    }
                    // a bump map's own normal map is what it is made from: over a game texture,
                    // published as it is, nothing reads it
                    if let Some(n) = t.asset.normal_map.as_ref().filter(|_| !t.as_is) {
                        named.push((vec![Seg::key("normal_map")], n.clone()));
                    }
                    if let Some(b) =
                        t.asset.bump.as_ref().filter(|b| b.source.is_none()).and_then(|b| b.texture.clone())
                    {
                        named.push((vec![Seg::key("bump"), Seg::key("texture")], b));
                    }
                    for (path, n) in named {
                        if !texture_known(root, index, game, &n) {
                            out.push(warn(NO_TEXTURE, path, format!("the texture '{n}' is nobody's: no texture asset, no file of the project, not the game's")));
                        }
                    }
                    if let Some(src) = t.asset.bump.as_ref().and_then(|b| b.source.as_deref()) {
                        let p = crate::files::beside(&root.join(rel.replace('/', "\\")), src);
                        if !crate::files::inside(root, &p) {
                            out.push(Issue::new(
                                resolve::OUTSIDE,
                                Severity::Error,
                                vec![Seg::key("bump"), Seg::key("source")],
                                format!("the bump picture lies outside the project: {src}"),
                            ));
                        } else if !p.is_file() {
                            out.push(Issue::new(
                                resolve::MISSING,
                                Severity::Error,
                                vec![Seg::key("bump"), Seg::key("source")],
                                format!("the bump picture is not there: {src}"),
                            ));
                        } else if let Err(e) = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| {
                            let ext =
                                p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                            gz_fmt_tex::load_image(&b, Some(&ext)).map(|_| ()).map_err(|e| e.to_string())
                        }) {
                            out.push(Issue::new(
                                resolve::UNREADABLE,
                                Severity::Error,
                                vec![Seg::key("bump"), Seg::key("source")],
                                format!("{src}: {e}"),
                            ));
                        }
                    }
                }
            }
        }
        MediaKind::Sound => match resolve::sound(root, rel) {
            Err(f) => out.push(f.issue),
            Ok((asset, source, _)) => {
                let at = if asset.source.is_some() { vec![Seg::key("source")] } else { Vec::new() };
                let read = std::fs::read(&source).map_err(|e| gz_base::io_reason(&e)).and_then(|b| {
                    if source.extension().is_some_and(|x| x.eq_ignore_ascii_case("ogg")) {
                        gz_fmt_sound::ogg_info(&b).map(|_| ()).map_err(|e| e.to_string())
                    } else {
                        gz_fmt_sound::read_wav(&b).map(|_| ()).map_err(|e| e.to_string())
                    }
                });
                if let Err(e) = read {
                    let name =
                        source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    out.push(Issue::new(resolve::UNREADABLE, Severity::Error, at, format!("{name}: {e}")));
                }
                out.extend(streamer_issues(root, index, game, &asset));
            }
        },
    }
    out
}

/// What the check of a texture asset reads of it: its keys, and its picture's - the header of
/// a game texture (`.dds`, published as it is), the decoded picture of any other.
struct TextureFacts {
    asset: crate::texture::TextureAsset,
    /// Its picture is a game texture published as it is (a `.dds`), or the game's own.
    as_is: bool,
}

/// The keys a texture asset states that say how a picture is made, each a warning at its key:
/// over a game texture published as it is nothing applies them.
fn making_keys_unread(value: &gz_luadata::Value, what: &str, out: &mut Vec<Issue>) {
    for k in crate::texture::MAKING_KEYS {
        if value.get(k).is_some() {
            out.push(Issue::new(
                resolve::CANNOT,
                Severity::Warning,
                vec![Seg::key(k)],
                format!("`{k}` says how a picture is made: {what}, published as it is, nothing applies it"),
            ));
        }
    }
}

/// A texture asset's facts for its check: a description of a texture of the game is told
/// against the game's list, a `.dds` by its header, any other picture decoded. Blocking.
fn texture_facts(
    root: &Path,
    rel: &str,
    name: &str,
    game: &GameLists,
    out: &mut Vec<Issue>,
) -> Result<TextureFacts, resolve::Failure> {
    let abs = root.join(rel.replace('/', "\\"));
    let text = std::fs::read_to_string(&abs).map_err(|e| {
        resolve::Failure::new(resolve::UNREADABLE, "", format!("{rel}: {}", gz_base::io_reason(&e)))
    })?;
    let value = crate::kind::load(MediaKind::Texture, rel, &text)
        .map_err(|e| resolve::Failure::new(resolve::UNREADABLE, "", e))?;
    let asset = crate::texture::TextureAsset::from_value(&value).0;
    if let Some(g) = crate::files::game_texture(asset.source.as_deref()) {
        // a description of the game's texture publishes under that texture's name, or nowhere
        if asset.name.as_deref().is_some_and(|n| crate::read::name_key(n) != crate::read::name_key(&g)) {
            out.push(Issue::new(
                resolve::CANNOT,
                Severity::Error,
                vec![Seg::key("name")],
                format!(
                    "a description of the game's texture '{g}' is that texture's: leave `name` out (it is '{g}'), or give the asset a picture of its own"
                ),
            ));
        }
        if game.linked && !game.has_texture(&g) {
            out.push(Issue::new(
                NO_TEXTURE,
                Severity::Warning,
                vec![Seg::key("source")],
                format!(
                    "the game has no texture '{g}': this describes a texture nobody has (`asset.find kind=texture` lists what there is)"
                ),
            ));
        }
        // nothing is made over the game's picture: a bump picture would bake into nothing
        if asset.bump.as_ref().is_some_and(|b| b.source.is_some()) {
            out.push(Issue::new(
                resolve::CANNOT,
                Severity::Error,
                vec![Seg::key("bump"), Seg::key("source")],
                format!(
                    "a description of the game's texture '{g}' makes no bump map from a picture: name a bump texture (`bump.texture`) instead"
                ),
            ));
        }
        making_keys_unread(&value, "the game's texture", out);
        return Ok(TextureFacts { asset, as_is: true });
    }
    let (asset, source, _) = resolve::texture_source(root, rel)?;
    if crate::files::is_game_texture_file(&source) {
        let bytes = crate::files::dds_head(&source).map_err(|e| {
            resolve::Failure::new(
                resolve::UNREADABLE,
                "source",
                format!("{}: {}", source.display(), gz_base::io_reason(&e)),
            )
        })?;
        let leaf = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        gz_fmt_tex::read_dds_info(&bytes)
            .map_err(|e| resolve::Failure::new(resolve::UNREADABLE, "source", format!("{leaf}: {e}")))?;
        making_keys_unread(&value, &format!("{leaf} is a game texture already"), out);
        return Ok(TextureFacts { asset, as_is: true });
    }
    let t = resolve::texture(root, rel, name)?;
    Ok(TextureFacts { asset: t.asset, as_is: false })
}

/// The problems of a sound's replacement in the streamer mode, at its `streamer` key: one on a
/// sound that is not copyrighted (`M114`), one that is copyrighted itself (`M115`), one nobody
/// has (`M116`). Blocking.
#[must_use]
pub fn streamer_issues(root: &Path, index: &Index, game: &GameLists, asset: &SoundAsset) -> Vec<Issue> {
    let Some(name) = asset.streamer.as_deref() else { return Vec::new() };
    let at = vec![Seg::key("streamer")];
    let mut out = Vec::new();
    if !asset.is_copyrighted() {
        out.push(Issue::new(
            STREAMER_UNUSED,
            Severity::Warning,
            at.clone(),
            format!("`streamer` names what plays instead of a copyrighted sound, and this sound is not marked `copyrighted = true`: '{name}' never plays for it"),
        ));
    }
    match crate::sound::mark(root, index, game, name) {
        crate::sound::Mark::Free => {}
        crate::sound::Mark::Copyrighted => out.push(Issue::new(
            STREAMER_COPYRIGHTED,
            Severity::Error,
            at,
            format!("the replacement '{name}' is copyrighted itself: the streamer mode keeps it from playing too - pick a sound without the mark"),
        )),
        crate::sound::Mark::Missing => out.push(Issue::new(
            STREAMER_MISSING,
            Severity::Error,
            at,
            format!("the replacement '{name}' is nobody's sound: no sound asset of the project publishes it and the linked game has none of that name"),
        )),
    }
    out
}

/// The check `gz-asset` runs for a media asset.
pub async fn check(ed: Arc<Editor>, file: Arc<AssetFile>, kind: MediaKind) -> Vec<Problem> {
    let Ok(media) = Media::of(&ed).await else { return Vec::new() };
    let index = media.fresh_index().await;
    let game = media.game_lists(&ed).await;
    let root = media.root().to_path_buf();
    let rel = file.path.as_str().to_owned();
    let value = file.doc.value();
    let models = crate::GameModels::reader(&ed, &root);
    let found = ed.exec.io(move || issues(&root, &rel, kind, &value, &index, &game, &models)).await;
    let mut seen: Vec<(String, Vec<Seg>, String)> = Vec::new();
    let mut out = Vec::new();
    for i in found {
        let key = (i.code.to_owned(), i.path.clone(), i.message.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let p = problem(&i);
        out.push(if i.path.is_empty() { file.place_head(p) } else { file.place(p, &i.path) });
    }
    out
}
