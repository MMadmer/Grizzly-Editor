//! `asset.import`: files of common formats brought into the project, each with its asset.
//!
//! A source is copied into the target folder (a `.gltf` with its buffers and pictures, an
//! `.obj` with its material libraries and their pictures, keeping where they lie beside it; the
//! pictures a `.glb` holds written out as `.png`), its asset is written beside it, and every
//! picture a model's surface wears gets a texture asset of its own, named in the model's
//! `surfaces`. What would be written is worked out first ([`plan`]); nothing is written when
//! the plan holds a file that is there and the caller did not say `overwrite`.

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
}

impl Plan {
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

/// What importing `from` into `to` (left out: each kind's folder) writes. `module` names the
/// textures a model's surfaces wear. Blocking.
#[must_use]
pub fn plan(from: &[PathBuf], to: Option<&RelPath>, module: &str) -> Plan {
    let mut plan = Plan::default();
    for src in from {
        let name = leaf(src);
        let Some(kind) = files::kind_of_source(&name) else {
            plan.problems.push(format!(
                "{name}: not a model (.glb, .gltf, .obj, .object), a picture (.png, .jpg, .tga, .bmp, .dds) or a sound (.wav, .ogg)"
            ));
            continue;
        };
        if !src.is_file() {
            plan.problems.push(format!("{}: there is no such file", src.display()));
            continue;
        }
        let folder = match to {
            Some(t) => t.clone(),
            None => RelPath::new(default_folder(kind)).unwrap_or_else(|_| RelPath::root()),
        };
        let Some(target) = join(&folder, &name) else {
            plan.problems.push(format!("{name}: the name cannot be a file of the project"));
            continue;
        };
        plan.add(target, Content::Copy(src.clone()), false);
        let asset = join(&folder, &format!("{}{}", stem(src), kind.extension()));
        let surfaces =
            if kind == MediaKind::Model { model_extras(&mut plan, src, &folder, module) } else { Vec::new() };
        if let Some(a) = asset {
            plan.add(a, Content::Bytes(asset_text(kind, &name, surfaces)), true);
        }
    }
    plan
}

/// A model's files beside it and the texture assets of its pictures; answers its `surfaces`.
fn model_extras(plan: &mut Plan, src: &Path, folder: &RelPath, module: &str) -> Vec<Value> {
    let dir = src.parent().unwrap_or(Path::new(""));
    let place = |p: &Path| -> Option<String> {
        let rel = p.strip_prefix(dir).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    };
    let deps = files::dependencies(src);
    for d in &deps {
        match place(d).and_then(|r| join(folder, &r)) {
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
                        plan.add(rel.clone(), Content::Copy(p.clone()), false);
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
            plan.add(t.clone(), Content::Bytes(asset_text(MediaKind::Texture, &file, Vec::new())), true);
            let mut row = Table::new();
            row.put("name", m.name.as_str());
            row.put("texture", files::published_name(None, t.as_str(), module));
            surfaces.push(Value::Table(row));
        }
    }
    surfaces
}
