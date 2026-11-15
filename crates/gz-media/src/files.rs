//! The files around a media asset: its source, what the source names beside it, the published
//! name, the module id, the stamps - plain reads, no state.

use std::path::{Path, PathBuf};

use gz_base::RelPath;
use gz_project::asset_kind::MediaKind;

/// The extensions a kind's source may have, lower case, without the dot.
#[must_use]
pub fn source_extensions(kind: MediaKind) -> &'static [&'static str] {
    match kind {
        MediaKind::Model => &["glb", "gltf", "obj", "object"],
        MediaKind::Texture => &["png", "jpg", "jpeg", "tga", "bmp", "dds"],
        MediaKind::Sound => &["wav", "ogg"],
    }
}

/// The kind a source file is of, by its extension.
#[must_use]
pub fn kind_of_source(name: &str) -> Option<MediaKind> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    MediaKind::ALL.into_iter().find(|k| source_extensions(*k).contains(&ext.as_str()))
}

/// FNV-1a: stable on every machine.
pub const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

#[must_use]
pub fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A file name of the asset's own text, relative to its folder: either slash, no climbing out
/// of the project is checked by the caller.
#[must_use]
pub fn beside(asset: &Path, file: &str) -> PathBuf {
    let dir = asset.parent().unwrap_or(Path::new(""));
    let mut p = dir.to_path_buf();
    for part in file.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".") {
        if part == ".." {
            p.pop();
        } else {
            p.push(part);
        }
    }
    p
}

/// The source of an asset: the file `source` names, else the file with the asset's own name and
/// one of the kind's extensions that is there. `None` when neither is.
#[must_use]
pub fn source_of(kind: MediaKind, asset: &Path, source: Option<&str>) -> Option<PathBuf> {
    if let Some(s) = source.filter(|s| !s.trim().is_empty()) {
        let p = beside(asset, s);
        return p.is_file().then_some(p);
    }
    source_extensions(kind).iter().map(|e| asset.with_extension(e)).find(|p| p.is_file())
}

/// What the source of an asset would be when it is there: the file `source` names, else the
/// first of the kind's extensions - the sentence of a missing source names it.
#[must_use]
pub fn expected_source(kind: MediaKind, asset: &Path, source: Option<&str>) -> String {
    match source.filter(|s| !s.trim().is_empty()) {
        Some(s) => s.to_owned(),
        None => {
            let stem = asset.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            source_extensions(kind).iter().map(|e| format!("{stem}.{e}")).collect::<Vec<_>>().join(", ")
        }
    }
}

/// The module id of a project: `[module] id` of its `mod.ltx`; empty without one. Blocking.
#[must_use]
pub fn module_id(root: &Path) -> String {
    match gz_mod::read_manifest_file(root) {
        Ok(Some(text)) => gz_mod::Manifest::parse(&gz_mod::ManifestDoc::parse(&text)).id,
        _ => String::new(),
    }
}

/// The name the game knows an asset by: its `name`, else `<module id>\<file name>`.
#[must_use]
pub fn published_name(name: Option<&str>, asset: &str, module: &str) -> String {
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        return n.replace('/', "\\");
    }
    let leaf = asset.rsplit(['/', '\\']).next().unwrap_or(asset);
    let stem = leaf.rfind('.').map_or(leaf, |d| &leaf[..d]);
    if module.is_empty() { stem.to_owned() } else { format!("{module}\\{stem}") }
}

/// The files a source names beside it, which travel with it: a glTF's buffers and pictures, an
/// OBJ's material libraries and their pictures. What is missing is left out. Blocking.
#[must_use]
pub fn dependencies(source: &Path) -> Vec<PathBuf> {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let dir = source.parent().unwrap_or(Path::new(""));
    let mut out: Vec<PathBuf> = Vec::new();
    let add = |rel: &str, out: &mut Vec<PathBuf>| {
        if rel.starts_with("data:") || rel.contains("://") || rel.trim().is_empty() {
            return;
        }
        let rel = percent_decode(rel);
        let p = beside(&dir.join("x"), &rel);
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    };
    match ext.as_str() {
        "gltf" => {
            let Ok(text) = std::fs::read_to_string(source) else { return out };
            let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
            for list in ["buffers", "images"] {
                for item in doc.get(list).and_then(|v| v.as_array()).into_iter().flatten() {
                    if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
                        add(uri, &mut out);
                    }
                }
            }
        }
        "obj" => {
            let Ok(text) = std::fs::read(source) else { return out };
            let text = String::from_utf8_lossy(&text);
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("mtllib ") {
                    let before = out.len();
                    add(rest.trim(), &mut out);
                    if out.len() > before
                        && let Some(mtl) = out.last().cloned()
                    {
                        for pic in mtl_pictures(&mtl) {
                            let at = beside(&mtl, &pic);
                            if at.is_file() && !out.contains(&at) {
                                out.push(at);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// The picture files a material library names (`map_Kd`, `map_Bump`, ...), as written.
fn mtl_pictures(mtl: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(mtl) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (key, rest) = l.split_once(char::is_whitespace)?;
            let key = key.to_ascii_lowercase();
            (key.starts_with("map_") || key == "bump" || key == "disp" || key == "decal")
                .then(|| rest.split_whitespace().last().unwrap_or("").to_owned())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// `%20` and the like of a URI.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A path of the project as the watcher and the index name it.
#[must_use]
pub fn rel_of(root: &Path, abs: &Path) -> Option<RelPath> {
    RelPath::from_fs(root, abs)
}
