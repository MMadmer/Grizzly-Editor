//! What the module build publishes of the media assets: for every one that ships, the data
//! baked from its source into the game's own containers, and a copy of the asset's text, under
//! the game's place for its name:
//!
//! ```text
//! texture   gamedata\textures\<name>.dds  (+ <name>_bump.dds, <name>_bump#.dds)  + <name>.texasset
//! sound     gamedata\sounds\<name>.ogg                                         + <name>.sndasset
//! dynamic   gamedata\meshes\<name>.ogf    (+ <name>.omf with motions)          + <name>.meshasset
//! ```
//!
//! What is baked is kept in `<project>\temp\media\` under a hash of the asset's text, its
//! name and every byte it is made of, so an unchanged asset costs a look at its files. A bake
//! is deterministic: the same files give the same bytes on every run and machine. The build
//! itself (`gz-build`) hands the files on through a [`Sink`].

use std::path::{Path, PathBuf};

use gz_fmt_ogf::ExportOptions;
use gz_fmt_tex::make_bump_textures;
use gz_project::asset_kind::MediaKind;

use crate::files::{self, FNV0, fnv};
use crate::index::{Entry, Index};
use crate::resolve;

/// Moves with every change of what a bake makes of the same files: old cached bakes are not
/// taken for new ones.
const BAKE_VERSION: &[u8] = b"gz-media bake 1";

/// A file the build writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Bytes(Vec<u8>),
    /// A baked file of the cache: the build may link it or copy it.
    File(PathBuf),
}

/// Where the published files go: a path relative to the module's root (back slashes) and its
/// content.
pub trait Sink {
    fn put(&mut self, rel: &str, payload: Payload) -> Result<(), String>;
}

impl<F: FnMut(&str, Payload) -> Result<(), String>> Sink for F {
    fn put(&mut self, rel: &str, payload: Payload) -> Result<(), String> {
        self(rel, payload)
    }
}

/// What a publish did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// The paths handed to the sink, in order.
    pub files: Vec<String>,
    /// One sentence per asset that could not be baked: its file and why.
    pub problems: Vec<String>,
    /// How many bakes came from the cache.
    pub cached: usize,
}

fn folder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Model => "gamedata\\meshes",
        MediaKind::Texture => "gamedata\\textures",
        MediaKind::Sound => "gamedata\\sounds",
    }
}

fn data_ext(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Model => "ogf",
        MediaKind::Texture => "dds",
        MediaKind::Sound => "ogg",
    }
}

/// The paths an asset that ships is published at (the build's `published_paths`). A texture's
/// bump pair and a model's motions are listed when the asset says it has them.
#[must_use]
pub fn paths_of(e: &Entry) -> Vec<String> {
    let base = format!("{}\\{}", folder(e.kind), e.name);
    let mut out = vec![format!("{base}.{}", data_ext(e.kind))];
    if e.kind == MediaKind::Texture && !e.uses.is_empty() {
        out.push(format!("{base}_bump.dds"));
        out.push(format!("{base}_bump#.dds"));
    }
    out.push(format!("{base}{}", e.kind.extension()));
    out
}

/// The cache folder of a project.
#[must_use]
pub fn cache_dir(root: &Path) -> PathBuf {
    root.join("temp").join("media")
}

/// The stamp of an asset's bake: its text, its name and every file it is made of.
fn bake_stamp(root: &Path, e: &Entry) -> u64 {
    let mut h = fnv(FNV0, BAKE_VERSION);
    h = fnv(h, e.name.to_lowercase().as_bytes());
    let mut all = vec![e.path.to_fs(root)];
    all.extend(e.source.iter().map(|s| s.to_fs(root)));
    all.extend(e.uses.iter().map(|s| s.to_fs(root)));
    for f in all {
        h = fnv(h, b"\0");
        if let Ok(b) = std::fs::read(f) {
            h = fnv(h, &b);
        }
    }
    h
}

/// A cached file, made by `make` when it is not there yet; written whole or not at all.
fn cached(
    dir: &Path,
    file: &str,
    hits: &mut usize,
    make: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let path = dir.join(file);
    if path.is_file() {
        *hits += 1;
        return Ok(path);
    }
    let bytes = make()?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
    gz_editor::fsx::write_atomic(&path, &bytes)
        .map_err(|e| format!("{}: {}", path.display(), gz_base::io_reason(&e)))?;
    Ok(path)
}

/// What a bake made of one asset.
#[derive(Default)]
struct Baked {
    /// `(path under the module, file of the cache)`.
    files: Vec<(String, PathBuf)>,
    /// The asset's text as the module ships it, when that is not the author's file as it is.
    text: Option<Vec<u8>>,
}

/// A model's text as the module ships it. The game reads row `n` of `surfaces` as the `n`th
/// child of the model, and the author writes rows by name, any of them, in any order: the
/// shipped text has one row per child of the baked model, in the children's order, each the
/// author's row of that surface with the texture and the shader the model was baked with. The
/// author's file as it is when its rows already say that.
fn shipped_model_text(text: &[u8], ogf: &[u8], m: &resolve::Model) -> Result<Option<Vec<u8>>, String> {
    use gz_luadata::{Table, Value};
    let visual = gz_fmt_ogf::Visual::read(ogf).map_err(|e| e.to_string())?;
    let children: Vec<(String, String)> = if visual.children.is_empty() {
        visual.texture.iter().map(|(t, s)| (t.to_string(), s.to_string())).collect()
    } else {
        visual
            .children
            .iter()
            .map(|c| c.texture.as_ref().map(|(t, s)| (t.to_string(), s.to_string())).unwrap_or_default())
            .collect()
    };
    let source = std::str::from_utf8(text).map_err(|e| e.to_string())?;
    let value = gz_luadata::parse(source).map_err(|e| e.message)?;
    let Some(top) = value.as_table() else { return Ok(None) };
    let said: Vec<&Table> =
        top.get("surfaces").map(|v| v.arr().iter().filter_map(Value::as_table).collect()).unwrap_or_default();
    let row_of = |name: &str| {
        said.iter()
            .find(|r| r.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name)))
    };
    let mut rows: Vec<Value> = Vec::with_capacity(children.len());
    for (texture, shader) in &children {
        // the asset's surface this child was made of: the first one baked with this look
        let name = m
            .object
            .surfaces
            .iter()
            .find(|s| s.texture.eq_ignore_ascii_case(texture) && s.shader.eq_ignore_ascii_case(shader))
            .map_or("", |s| s.name.as_str());
        let mut row = Table::new();
        row.put("name", name);
        row.put("texture", texture.as_str());
        row.put("shader", shader.as_str());
        if let Some(theirs) = row_of(name) {
            for (k, v) in &theirs.fields {
                if !matches!(k.as_str(), "name" | "texture" | "shader") {
                    row.put(k, v.clone());
                }
            }
        }
        rows.push(Value::Table(row));
    }
    let new = Value::Table(Table { arr: rows, fields: Vec::new() });
    // the author's rows already are the children's, in their order: the file as it is
    let same = said.len() == children.len()
        && said.iter().zip(&children).all(|(r, (texture, shader))| {
            let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default();
            text("texture").eq_ignore_ascii_case(texture) && text("shader").eq_ignore_ascii_case(shader)
        });
    if same {
        return Ok(None);
    }
    let mut out = Table::new();
    let mut put = false;
    for (k, v) in &top.fields {
        if k == "surfaces" {
            out.put(k, new.clone());
            put = true;
        } else {
            out.put(k, v.clone());
        }
    }
    if !put {
        out.put("surfaces", new);
    }
    Ok(Some(gz_luadata::write_asset("", &out).into_bytes()))
}

/// The baked files of one asset and, for a model, the text it ships with.
fn bake(root: &Path, e: &Entry, hits: &mut usize) -> Result<Baked, String> {
    let dir = cache_dir(root);
    let h = bake_stamp(root, e);
    let base = format!("{}\\{}", folder(e.kind), e.name);
    let rel = e.path.as_str();
    let mut out = Vec::new();
    let mut shipped = None;
    match e.kind {
        MediaKind::Texture => {
            let t = resolve::texture(root, rel, &e.name).map_err(|f| f.to_string())?;
            let dds = cached(&dir, &format!("{h:016x}.dds"), hits, || {
                gz_fmt_tex::make_game_texture(&t.image, &t.params, None)
                    .map(|g| g.dds)
                    .map_err(|err| err.to_string())
            })?;
            out.push((format!("{base}.dds"), dds));
            if let Some(src) = t.asset.bump.as_ref().and_then(|b| b.source.as_deref()) {
                let abs = files::beside(&e.path.to_fs(root), src);
                if !files::inside(root, &abs) {
                    return Err(format!("the bump picture lies outside the project: {src}"));
                }
                let from_normal = t.asset.bump.as_ref().and_then(|b| b.from) == Some(1);
                let depth = t.params.bump_virtual_height;
                let pair = std::cell::OnceCell::new();
                let make = || -> Result<gz_fmt_tex::BumpTextures, String> {
                    let bytes = std::fs::read(&abs)
                        .map_err(|err| format!("{}: {}", abs.display(), gz_base::io_reason(&err)))?;
                    let ext = abs.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                    let pic =
                        gz_fmt_tex::load_image(&bytes, Some(&ext)).map_err(|err| err.to_string())?.image;
                    if from_normal {
                        // a normal map alone: the height is flat, the gloss full
                        let flat = gz_fmt_tex::Image::filled(pic.width, pic.height, [128, 128, 128, 255])
                            .map_err(|err| err.to_string())?;
                        make_bump_textures(&flat, Some(&pic), depth).map_err(|err| err.to_string())
                    } else {
                        make_bump_textures(&pic, None, depth).map_err(|err| err.to_string())
                    }
                };
                let get = || -> Result<gz_fmt_tex::BumpTextures, String> {
                    if pair.get().is_none() {
                        let _ = pair.set(make()?);
                    }
                    pair.get().cloned().ok_or_else(|| "the bump map was not made".to_owned())
                };
                let bump = cached(&dir, &format!("{h:016x}_bump.dds"), hits, || get().map(|b| b.bump))?;
                let error = cached(&dir, &format!("{h:016x}_bump#.dds"), hits, || get().map(|b| b.error))?;
                out.push((format!("{base}_bump.dds"), bump));
                out.push((format!("{base}_bump#.dds"), error));
            }
        }
        MediaKind::Sound => {
            let (asset, source, _) = resolve::sound(root, rel).map_err(|f| f.to_string())?;
            let ogg = cached(&dir, &format!("{h:016x}.ogg"), hits, || {
                let thm = asset.to_thm();
                let params = gz_fmt_sound::OggParams::from(&thm);
                let bytes = std::fs::read(&source)
                    .map_err(|err| format!("{}: {}", source.display(), gz_base::io_reason(&err)))?;
                let is_ogg = source.extension().is_some_and(|x| x.eq_ignore_ascii_case("ogg"));
                if is_ogg {
                    gz_fmt_sound::write_ogg_params(&bytes, &params).map_err(|err| err.to_string())
                } else {
                    let wav = gz_fmt_sound::read_wav(&bytes).map_err(|err| err.to_string())?;
                    gz_fmt_sound::encode_game_sound(&wav, thm.quality, &params).map_err(|err| err.to_string())
                }
            })?;
            out.push((format!("{base}.ogg"), ogg));
        }
        MediaKind::Model => {
            let m = resolve::model(root, rel).map_err(|f| f.to_string())?;
            // what its surfaces were resolved to is part of the model: a surface without a row
            // takes its texture from a texture asset's name and the module's id, which no file
            // of the stamp says
            let h = m.object.surfaces.iter().fold(h, |h, s| {
                [s.name.as_str(), s.texture.as_str(), s.shader.as_str()]
                    .iter()
                    .fold(fnv(h, b"\0"), |h, x| fnv(fnv(h, x.to_lowercase().as_bytes()), b"|"))
            });
            let opts = ExportOptions { source_file: e.name.as_str().into(), ..ExportOptions::default() };
            let ogf = cached(&dir, &format!("{h:016x}.ogf"), hits, || {
                gz_fmt_ogf::export_skeleton(&m.object, &opts, 4)
                    .map(|x| x.bytes)
                    .map_err(|err| err.to_string())
            })?;
            let baked = std::fs::read(&ogf)
                .map_err(|err| format!("{}: {}", ogf.display(), gz_base::io_reason(&err)))?;
            let text = std::fs::read(e.path.to_fs(root))
                .map_err(|err| format!("{}: {}", e.path, gz_base::io_reason(&err)))?;
            shipped = shipped_model_text(&text, &baked, &m)?;
            out.push((format!("{base}.ogf"), ogf));
            if !m.object.motions.is_empty() {
                let omf = cached(&dir, &format!("{h:016x}.omf"), hits, || {
                    gz_fmt_ogf::export_omf(&m.object).map(|x| x.bytes).map_err(|err| err.to_string())
                })?;
                out.push((format!("{base}.omf"), omf));
            }
        }
    }
    Ok(Baked { files: out, text: shipped })
}

/// Publishes every asset of the index that ships into `sink`: its baked files from the cache,
/// then its text. An asset that cannot be baked is a problem and publishes nothing; the others
/// go on. Blocking: run it on the IO pool.
pub fn publish(root: &Path, index: &Index, sink: &mut dyn Sink) -> Published {
    let mut done = Published::default();
    for e in index.entries.iter().filter(|e| e.ships()) {
        let baked = match bake(root, e, &mut done.cached) {
            Ok(b) => b,
            Err(why) => {
                done.problems.push(format!("{}: {why}", e.path));
                continue;
            }
        };
        let text = match baked.text {
            Some(t) => t,
            None => match std::fs::read(e.path.to_fs(root)) {
                Ok(t) => t,
                Err(err) => {
                    done.problems.push(format!("{}: {}", e.path, gz_base::io_reason(&err)));
                    continue;
                }
            },
        };
        let mut put = |rel: String, payload: Payload, done: &mut Published| match sink.put(&rel, payload) {
            Ok(()) => done.files.push(rel),
            Err(why) => done.problems.push(format!("{rel}: {why}")),
        };
        for (rel, file) in baked.files {
            put(rel, Payload::File(file), &mut done);
        }
        put(format!("{}\\{}{}", folder(e.kind), e.name, e.kind.extension()), Payload::Bytes(text), &mut done);
    }
    done
}
