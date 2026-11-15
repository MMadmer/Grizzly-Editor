//! Mesh sources of common formats read into the editor's source model.
//!
//! ```text
//! read(path)      -> MeshSource     by the extension: .glb / .gltf / .obj
//! read_gltf(path) -> MeshSource     glTF 2.0: a .gltf with external or data-URI buffers, or a .glb
//! read_obj(path)  -> MeshSource     Wavefront OBJ with its .mtl
//! ```
//!
//! A source becomes the same [`Object`] a library `.object` is, so everything downstream - the
//! picture, the rays, the level compiler, the OGF export - is one code path. What a format holds
//! beyond geometry (materials, pictures) comes beside it in the [`MeshSource`]; nothing here
//! decides a texture name or a shader the game knows: the asset layer above does.
//!
//! # Space
//!
//! Both formats are right-handed with +Y up and metres; the game is left-handed with +Y up and
//! metres. A point `(x, y, z)` becomes `(x, y, -z)`, a normal likewise, and every triangle's
//! winding is reversed so it still faces the same way. Seen from above, a source and its result look
//! alike: the source's -Z ("away") is the game's +Z. Node rotations are mirrored the same way
//! (`S * M * S` with `S = diag(1, 1, -1)`). Texture coordinates: glTF and the game both have v
//! going down, so a glTF UV is kept; OBJ has v going up, so its v becomes `1 - v`.
//!
//! # What comes out
//!
//! - One [`Mesh`](gz_fmt_object::Mesh) per glTF node that holds a mesh, or per OBJ object or
//!   group; positions welded within it (corners at the same point share it).
//! - One surface per material (a triangle without one gets `default`), each valid as it is:
//!   shader and compiler shader `default`, game material `default` (the name the level
//!   compiler finds in the game's material list), the vertex map `Texture` and exactly one texture
//!   coordinate set. The texture is left empty: [`MeshSource::materials`] (parallel to
//!   `object.surfaces`) says which picture a surface wears.
//! - Hard edges where the source's normals differ across an edge (or, in OBJ without normals,
//!   where the smoothing groups do): the per-face edge flags of the model.
//! - glTF skins: a dynamic object with one bone per joint (parent first; the bind pose from the
//!   inverse bind matrices), the four heaviest weights of every vertex brought to a sum of one, and
//!   one bone part `default` holding every bone. Every animation clip is a motion of its own,
//!   sampled at 30 keys a second. A mesh moved by an animation without a skin is a rigid
//!   skeleton: one bone per node.
//!
//! Hostile input is an error, never a panic or an endless loop; element counts are capped at
//! [`MAX_ELEMENTS`], files at [`MAX_FILE_BYTES`].

mod assemble;
mod gltf;
mod math;
mod obj;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use gz_fmt_object::Object;

pub use gltf::read_gltf;
pub use obj::read_obj;

/// The extensions this crate reads, lower case, without the dot.
pub const EXTENSIONS: &[&str] = &["glb", "gltf", "obj"];

/// The most elements an accessor, a vertex list or a face list may hold.
pub const MAX_ELEMENTS: usize = 1 << 24;

/// The largest file (and glTF buffer) read.
pub const MAX_FILE_BYTES: u64 = 1 << 30;

/// The most nodes a glTF document may hold.
pub const MAX_NODES: usize = 1 << 20;

/// The most keys a sampled motion may hold: an hour at 30 a second.
pub const MAX_FRAMES: usize = 30 * 60 * 60;

/// Motions are sampled at this rate.
pub const SAMPLE_FPS: f32 = 30.0;

/// A mesh source read into the model.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSource {
    pub object: Object,
    /// One per surface of `object`, in the same order.
    pub materials: Vec<SourceMaterial>,
    pub pictures: Vec<SourcePicture>,
    /// What was read but has no place in the model (vertex colours, morph targets, lines, ...),
    /// one sentence each.
    pub notes: Vec<String>,
}

/// What a source says about a surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMaterial {
    /// The material's name in the source; also the surface's name, made unique.
    pub name: String,
    /// Index into [`MeshSource::pictures`]: the base colour picture.
    pub picture: Option<usize>,
    pub two_sided: bool,
}

/// A picture a source names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePicture {
    pub name: String,
    /// `image/png`, `image/jpeg`, ...; empty when nothing says.
    pub mime: String,
    pub data: PictureData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureData {
    /// The bytes are inside the source (a glTF buffer or data URI).
    Embedded(Vec<u8>),
    /// A file beside the source, resolved against its folder; it may not exist.
    File(PathBuf),
}

/// Why a source could not be read: a sentence for a person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// A file could not be read.
    Io { path: PathBuf, reason: String },
    /// The file is not what it should be.
    Invalid { file: String, message: String },
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, reason } => write!(f, "{} could not be read: {reason}.", path.display()),
            Self::Invalid { file, message } => write!(f, "{file}: {message}."),
        }
    }
}

impl std::error::Error for MeshError {}

pub type Result<T, E = MeshError> = std::result::Result<T, E>;

/// An error of a file's content, before it is tied to the file.
pub(crate) type Bad = String;

/// Reads a source by its extension.
pub fn read(path: &Path) -> Result<MeshSource> {
    match extension(path).as_str() {
        "glb" | "gltf" => read_gltf(path),
        "obj" => read_obj(path),
        other => Err(MeshError::Invalid {
            file: file_name(path),
            message: format!(
                "`.{other}` is no mesh source this editor reads (it reads {})",
                EXTENSIONS.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", ")
            ),
        }),
    }
}

pub(crate) fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// A file a document names, beside the document: a relative name, either slash. `None` for a
/// name that is rooted or carries a prefix - a drive, a server (`C:\x`, `/x`, `\\host\share`): a
/// model names its buffers, its material library and its pictures relative to itself, and a
/// name that points at a place of the machine or the network is not followed (opening one
/// would read whatever is there, or ask a server for it).
pub(crate) fn beside(dir: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    let rooted = name.starts_with(['/', '\\']) || name.split(['/', '\\']).any(|part| part.contains(':'));
    if name.is_empty() || rooted {
        return None;
    }
    let mut path = dir.to_path_buf();
    for part in name.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".") {
        if part == ".." {
            path.pop();
        } else {
            path.push(part);
        }
    }
    Some(path)
}

/// Reads a whole file, refusing one larger than [`MAX_FILE_BYTES`].
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    let io = |e: std::io::Error| MeshError::Io { path: path.to_owned(), reason: gz_base::io_reason(&e) };
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_FILE_BYTES {
        return Err(MeshError::Invalid {
            file: file_name(path),
            message: format!("the file is {len} bytes long; the most this editor reads is {MAX_FILE_BYTES}"),
        });
    }
    std::fs::read(path).map_err(io)
}

/// The MIME type of a picture file by its extension.
pub(crate) fn mime_of(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("tga") => "image/x-tga",
        Some("bmp") => "image/bmp",
        Some("dds") => "image/vnd-ms.dds",
        Some("webp") => "image/webp",
        Some("ktx2") => "image/ktx2",
        _ => "",
    }
}
