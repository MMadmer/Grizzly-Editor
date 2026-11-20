//! The files around a media asset: its source, what the source names beside it, the published
//! name, the module id, the stamps - plain reads, no state.

use std::path::{Path, PathBuf};

use gz_base::RelPath;
use gz_project::asset_kind::MediaKind;

/// The extensions a kind's source may have, lower case, without the dot.
#[must_use]
pub fn source_extensions(kind: MediaKind) -> &'static [&'static str] {
    match kind {
        MediaKind::Model => &["glb", "gltf", "obj", "object", "ogf"],
        MediaKind::Texture => &["png", "jpg", "jpeg", "tga", "bmp", "dds"],
        MediaKind::Sound => &["wav", "ogg"],
    }
}

/// The head of a `.dds`: what its header says is all in its first 148 bytes. Blocking.
pub fn dds_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::with_capacity(148);
    std::fs::File::open(path)?.take(148).read_to_end(&mut head)?;
    Ok(head)
}

/// The kind a source file is of, by its extension.
#[must_use]
pub fn kind_of_source(name: &str) -> Option<MediaKind> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    MediaKind::ALL.into_iter().find(|k| source_extensions(*k).contains(&ext.as_str()))
}

/// What a model's `source` starts with when it names a compiled model of the game.
pub const GAME_PREFIX: &str = "game:";

/// The engine name of the game's model a `source` names (`game:dynamics/box/box_wood_01.ogf`
/// is `dynamics\box\box_wood_01`), back slashes, lower case, without the extension. `None` for a
/// source that is a file of the project.
#[must_use]
pub fn game_model(source: Option<&str>) -> Option<String> {
    let name = source?.trim().strip_prefix(GAME_PREFIX)?.trim().replace('/', "\\").to_ascii_lowercase();
    Some(name.strip_suffix(".ogf").unwrap_or(&name).trim_matches('\\').to_owned())
}

/// The game name of the texture a `source` names (`game:wpn/wpn_ak74.dds` is `wpn\wpn_ak74`),
/// back slashes, lower case, without the extension: the asset describes a texture of the game,
/// whose picture stays the game's. `None` for a source that is a file of the project.
#[must_use]
pub fn game_texture(source: Option<&str>) -> Option<String> {
    let name = source?.trim().strip_prefix(GAME_PREFIX)?.trim().replace('/', "\\").to_ascii_lowercase();
    let name = name.strip_suffix(".dds").unwrap_or(&name).trim_matches('\\').to_owned();
    Some(name)
}

/// Whether a texture's source is a game texture already (`.dds`): it is published as it is,
/// never decoded and made again.
#[must_use]
pub fn is_game_texture_file(source: &Path) -> bool {
    source.extension().is_some_and(|e| e.eq_ignore_ascii_case("dds"))
}

/// The `#` companion of a game texture (`<stem>#.dds` beside `<stem>.dds`): the second texture
/// of a bump pair, published with it. `None` when there is none, and when the companion has a
/// texture asset of its own beside it (`<stem>#.texasset`): it is published as that asset.
#[must_use]
pub fn hash_companion(source: &Path) -> Option<PathBuf> {
    if !is_game_texture_file(source) {
        return None;
    }
    let stem = source.file_stem()?.to_string_lossy().into_owned();
    let p = source.with_file_name(format!("{stem}#.dds"));
    let own = source.with_file_name(format!("{stem}#{}", MediaKind::Texture.extension()));
    (p.is_file() && !own.is_file()).then_some(p)
}

/// The folder of the game's data a kind's files lie under (`textures`, `sounds`, `meshes`).
#[must_use]
pub fn game_folder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Model => "meshes",
        MediaKind::Texture => "textures",
        MediaKind::Sound => "sounds",
    }
}

/// The name the game knows a file by when it lies in a tree of the game's data - any path with
/// `gamedata\<textures|sounds|meshes>\` in it (`.../gamedata/textures/wpn/ak.dds` is
/// `wpn\ak`) -, without its extension, back slashes. `None` for a file of no such tree.
#[must_use]
pub fn game_name_of(kind: MediaKind, path: &str) -> Option<String> {
    let p = path.replace('\\', "/");
    let lower = p.to_ascii_lowercase();
    let marker = format!("gamedata/{}/", game_folder(kind));
    let at = if lower.starts_with(&marker) { 0 } else { lower.find(&format!("/{marker}"))? + 1 };
    let rest = &p[at + marker.len()..];
    let leaf = rest.rsplit('/').next().unwrap_or(rest);
    let stem_len = leaf.rfind('.').map_or(rest.len(), |d| rest.len() - leaf.len() + d);
    let name = rest[..stem_len].trim_matches('/').replace('/', "\\");
    (!name.is_empty()).then_some(name)
}

/// Whether the game can ask for a texture by this name: its loader drops one `.dds`, `.tga`,
/// `.bmp` or `.ogm` from the end of a name it is given, and no config gives a name that starts or
/// ends with a space. So a file `<name>.dds.dds` or `<name> .dds` is a file nothing loads.
#[must_use]
pub fn texture_name_reached(name: &str) -> bool {
    if name.trim() != name || name.is_empty() {
        return false;
    }
    let leaf = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let ext = leaf.rfind('.').map(|d| leaf[d + 1..].to_ascii_lowercase());
    !matches!(ext.as_deref(), Some("dds" | "tga" | "bmp" | "ogm"))
}

/// Whether an engine name is a plain name under `meshes`: no drive, no root, no step up.
#[must_use]
pub fn plain_game_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(':')
        && name.split('\\').all(|part| !part.is_empty() && part != "." && part != "..")
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

/// A file name of the asset's own text, relative to its folder: either slash. A name is never
/// rooted - a drive or a server in it is no part of the path, so `C:/x` and `\\host\x` are
/// files beside the asset like any other -, and whether `..` climbs out of the project is the
/// caller's check ([`inside`]).
#[must_use]
pub fn beside(asset: &Path, file: &str) -> PathBuf {
    let dir = asset.parent().unwrap_or(Path::new(""));
    let mut p = dir.to_path_buf();
    for part in file.split(['/', '\\']).filter(|s| !s.is_empty() && *s != "." && !s.contains(':')) {
        if part == ".." {
            p.pop();
        } else {
            p.push(part);
        }
    }
    p
}

/// Whether `path` lies in the project at `root`, as both are written (any case): a project names
/// nothing outside it.
#[must_use]
pub fn inside(root: &Path, path: &Path) -> bool {
    let lower = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let (root, path) = (lower(root), lower(path));
    let root = root.trim_end_matches('\\');
    path.strip_prefix(root).is_some_and(|rest| rest.starts_with('\\'))
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

/// The name the game knows an asset by: its `name`; else, for an asset that lies in the
/// module's own `gamedata\<textures|sounds|meshes>\`, the path below that folder (an asset
/// beside a file of the game's data names what lies there); else `<module id>\<file name>`.
#[must_use]
pub fn published_name(name: Option<&str>, asset: &str, module: &str) -> String {
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        return n.replace('/', "\\");
    }
    let lower = asset.replace('\\', "/").to_ascii_lowercase();
    for kind in MediaKind::ALL {
        if lower.starts_with(&format!("gamedata/{}/", game_folder(kind)))
            && lower.ends_with(kind.extension())
            && let Some(n) = game_name_of(kind, asset)
        {
            return n;
        }
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
