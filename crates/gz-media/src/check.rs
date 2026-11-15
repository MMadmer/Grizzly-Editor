//! `asset.check` of a media asset: every problem at the key it is about.
//!
//! | Code | |
//! |---|---|
//! | `M101` | the source is not there |
//! | `M102` | the source cannot be read (the reader's sentence) |
//! | `M103` | two assets of a kind publish one name (on both) |
//! | `M104` | a texture nobody has: no texture asset, no file of the project, not the game's (warning) |
//! | `M105` `M106` `M107` | an engine shader, a compiler shader, a game material the linked game lacks (warning) |
//! | `M108` | a dynamic model without bones gets one (note) |
//! | `M109` | a value outside what its key takes |
//! | `M110` | `parts` together with `source` |
//! | `M111` | what the model cannot take (warning) |

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
        MediaKind::Model => match resolve::model(root, rel) {
            Err(f) => out.push(f.issue),
            Ok(m) => {
                out.extend(m.issues.into_iter().filter(|i| i.code != crate::read::BAD_VALUE));
                let asset = &m.asset;
                for s in &m.object.surfaces {
                    let row = asset.surfaces.iter().find(|r| r.name.eq_ignore_ascii_case(&s.name));
                    let at = |key: &str| match row {
                        Some(r) => vec![Seg::key("surfaces"), Seg::item("name", &r.name), Seg::key(key)],
                        None => vec![Seg::key("surfaces")],
                    };
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
                    if !s.game_mtl.is_empty() && !game.materials.contains(&s.game_mtl.to_lowercase()) {
                        out.push(warn(
                            NO_MATERIAL,
                            at("material"),
                            format!("the linked game has no game material '{}'", s.game_mtl),
                        ));
                    }
                }
                if game.linked {
                    for b in &asset.bones {
                        if let Some(mtl) = &b.material
                            && !game.materials.contains(&mtl.to_lowercase())
                        {
                            out.push(warn(
                                NO_MATERIAL,
                                vec![Seg::key("bones"), Seg::item("name", &b.name), Seg::key("material")],
                                format!("the linked game has no game material '{mtl}'"),
                            ));
                        }
                    }
                }
            }
        },
        MediaKind::Texture => {
            let name = index.at(rel).map(|e| e.name.clone()).unwrap_or_default();
            match resolve::texture(root, rel, &name) {
                Err(f) => out.push(f.issue),
                Ok(t) => {
                    let mut named: Vec<(Vec<Seg>, String)> = Vec::new();
                    if let Some(d) = t.asset.detail.as_ref().and_then(|d| d.texture.clone()) {
                        named.push((vec![Seg::key("detail"), Seg::key("texture")], d));
                    }
                    if let Some(n) = &t.asset.normal_map {
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
                        let p = crate::files::beside(&t.source.with_file_name("x"), src);
                        if !p.is_file() {
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
            }
        },
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
    let found = ed.exec.io(move || issues(&root, &rel, kind, &value, &index, &game)).await;
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
