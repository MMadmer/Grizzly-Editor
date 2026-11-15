//! What the texture preview loads: a file's bytes wherever it lives, taken by the GPU as they
//! are when it can, decoded otherwise. A texture asset of the project is shown as the game will
//! have it: its picture baked into the `.dds` the module build publishes, with the asset's keys.

use std::path::PathBuf;
use std::sync::Arc;

use gz_editor::Editor;
use gz_fmt_tex::Image;
use gz_vfs::Source;

use crate::loader::Sources;

/// Where a texture of each browser source is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexSource {
    /// A file of the project, by its project-relative path.
    Project,
    /// The editor's library: a name without an extension, `$game_textures$` (`.dds`), then
    /// `$textures$` (`.tga`, then `.dds`).
    Editor,
    /// The linked game: a loose file under its gamedata, or one inside an archive.
    Game,
}

impl TexSource {
    /// `project` (or nothing), `library` or `game`; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "" | "project" => Some(TexSource::Project),
            "library" => Some(TexSource::Editor),
            "game" => Some(TexSource::Game),
            _ => None,
        }
    }

    #[must_use]
    pub fn arg(self) -> &'static str {
        match self {
            TexSource::Project => "project",
            TexSource::Editor => "library",
            TexSource::Game => "game",
        }
    }
}

/// A pixel format the preview names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    pub name: &'static str,
    /// Bytes per 4 x 4 block (compressed) or per pixel.
    pub block: u32,
    pub compressed: bool,
    pub alpha: bool,
    pub srgb: bool,
}

const fn fmt(name: &'static str, block: u32, compressed: bool, alpha: bool, srgb: bool) -> FormatInfo {
    FormatInfo { name, block, compressed, alpha, srgb }
}

/// A DXGI format the GPU takes as it is, what it is named, and the format it is uploaded as.
#[must_use]
pub fn dxgi(code: u32) -> Option<(FormatInfo, wgpu::TextureFormat)> {
    use wgpu::TextureFormat as F;
    Some(match code {
        71 => (fmt("BC1 (DXT1)", 8, true, false, false), F::Bc1RgbaUnorm),
        72 => (fmt("BC1 sRGB", 8, true, false, true), F::Bc1RgbaUnorm),
        74 => (fmt("BC2 (DXT3)", 16, true, true, false), F::Bc2RgbaUnorm),
        75 => (fmt("BC2 sRGB", 16, true, true, true), F::Bc2RgbaUnorm),
        77 => (fmt("BC3 (DXT5)", 16, true, true, false), F::Bc3RgbaUnorm),
        78 => (fmt("BC3 sRGB", 16, true, true, true), F::Bc3RgbaUnorm),
        80 => (fmt("BC4 (ATI1)", 8, true, false, false), F::Bc4RUnorm),
        83 => (fmt("BC5 (ATI2)", 16, true, false, false), F::Bc5RgUnorm),
        95 => (fmt("BC6H", 16, true, false, false), F::Bc6hRgbUfloat),
        98 => (fmt("BC7", 16, true, true, false), F::Bc7RgbaUnorm),
        99 => (fmt("BC7 sRGB", 16, true, true, true), F::Bc7RgbaUnorm),
        87 => (fmt("B8G8R8A8", 4, false, true, false), F::Bgra8Unorm),
        91 => (fmt("B8G8R8A8 sRGB", 4, false, true, true), F::Bgra8Unorm),
        28 => (fmt("R8G8B8A8", 4, false, true, false), F::Rgba8Unorm),
        29 => (fmt("R8G8B8A8 sRGB", 4, false, true, true), F::Rgba8Unorm),
        88 => (fmt("B8G8R8X8", 4, false, false, false), F::Bgra8Unorm),
        10 => (fmt("R16G16B16A16F", 8, false, true, false), F::Rgba16Float),
        61 => (fmt("R8", 1, false, false, false), F::R8Unorm),
        _ => return None,
    })
}

/// What the tab says of a file it could not show; the command answers [`UNREADABLE_REPLY`].
pub const UNREADABLE: &str = "The file cannot be decoded as an image.";
pub const UNREADABLE_REPLY: &str = "cannot decode this file as an image";

/// What the decoder makes of a file the GPU cannot take: its top level as B8G8R8A8.
pub const DECODED: u32 = 87;

/// A DDS the GPU takes as it is: its format, its size and the levels it holds whole.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeDds {
    pub dxgi: u32,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

fn u32_at(p: &[u8], at: usize) -> u32 {
    p.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// `NativeDds`: a block-compressed or 32-bit 2D texture is taken in its own format with every
/// level it holds; anything else (a cube map, a volume, 24 or 16 bits) is `None`, left to the
/// decoder. A file cut short keeps the levels it holds whole.
#[must_use]
pub fn native_dds(p: &[u8]) -> Option<NativeDds> {
    if p.len() < 128 || p.get(..4) != Some(b"DDS ".as_slice()) {
        return None;
    }
    let (flags, height, width, mip_count) = (u32_at(p, 8), u32_at(p, 12), u32_at(p, 16), u32_at(p, 28));
    let (pf_flags, four_cc, bits) = (u32_at(p, 80), u32_at(p, 84), u32_at(p, 88));
    let (rmask, gmask, bmask, amask) = (u32_at(p, 92), u32_at(p, 96), u32_at(p, 100), u32_at(p, 104));
    let caps2 = u32_at(p, 112);
    const CUBE: u32 = 0x200;
    const VOLUME: u32 = 0x20_0000;
    const FOURCC: u32 = 0x4;
    const RGB: u32 = 0x40;
    const ALPHA_PIXELS: u32 = 0x1;
    const MIP_COUNT: u32 = 0x2_0000;
    if width == 0 || height == 0 || width > 16384 || height > 16384 || caps2 & (CUBE | VOLUME) != 0 {
        return None;
    }
    let cc = |s: &[u8; 4]| u32::from_le_bytes(*s);
    let mut data = 128usize;
    let code = if pf_flags & FOURCC != 0 {
        match four_cc {
            x if x == cc(b"DXT1") => 71,
            x if x == cc(b"DXT2") || x == cc(b"DXT3") => 74,
            x if x == cc(b"DXT4") || x == cc(b"DXT5") => 77,
            x if x == cc(b"ATI1") || x == cc(b"BC4U") => 80,
            x if x == cc(b"ATI2") || x == cc(b"BC5U") => 83,
            x if x == cc(b"DX10") => {
                // DDS_HEADER_DXT10: format, dimension (3 = 2D), misc (4 = cube), array size
                if p.len() < 148 || u32_at(p, 132) != 3 || u32_at(p, 136) & 0x4 != 0 || u32_at(p, 140) > 1 {
                    return None;
                }
                data = 148;
                u32_at(p, 128)
            }
            _ => return None,
        }
    } else if pf_flags & RGB != 0 && bits == 32 {
        let alpha = pf_flags & ALPHA_PIXELS != 0 && amask == 0xFF00_0000;
        if rmask == 0x00FF_0000 && gmask == 0x0000_FF00 && bmask == 0x0000_00FF {
            if alpha { 87 } else { 88 }
        } else if rmask == 0x0000_00FF && gmask == 0x0000_FF00 && bmask == 0x00FF_0000 && alpha {
            28
        } else {
            return None;
        }
    } else {
        return None;
    };
    let (fi, _) = dxgi(code)?;
    // a block-compressed texture's top level is whole blocks
    if fi.compressed && (width & 3 != 0 || height & 3 != 0) {
        return None;
    }
    let full = 32 - width.max(height).leading_zeros();
    let mips = if flags & MIP_COUNT != 0 && mip_count != 0 { mip_count } else { 1 }.clamp(1, full);
    let mut levels = Vec::with_capacity(mips as usize);
    let mut offset = data;
    for m in 0..mips {
        let (w, h) = ((width >> m).max(1), (height >> m).max(1));
        let (pitch, rows) = if fi.compressed {
            (w.div_ceil(4).max(1) * fi.block, h.div_ceil(4).max(1))
        } else {
            (w * fi.block, h)
        };
        let len = pitch as usize * rows as usize;
        let Some(level) = p.get(offset..offset + len) else { break };
        let mut level = level.to_vec();
        if code == 88 {
            // X8 is no alpha: the view must not blend by it
            for px in level.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
        levels.push(level);
        offset += len;
    }
    if levels.is_empty() {
        return None;
    }
    Some(NativeDds { dxgi: code, width, height, levels })
}

/// What kind of file the bytes are, by their magic; TGA has none and is named by its extension.
#[must_use]
pub fn container_of(p: &[u8], name: &str) -> &'static str {
    if p.starts_with(b"DDS ") {
        "DDS"
    } else if p.len() >= 8 && p.starts_with(b"\x89PNG") {
        "PNG"
    } else if p.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "JPEG"
    } else if p.starts_with(b"BM") {
        "BMP"
    } else if name.to_lowercase().ends_with(".tga") {
        "TGA"
    } else {
        "image"
    }
}

/// Bytes the whole chain takes on the GPU: block formats round every level up to whole blocks
///.
#[must_use]
pub fn gpu_bytes(fi: &FormatInfo, width: u32, height: u32, mips: u32) -> u64 {
    (0..mips)
        .map(|m| {
            let (w, h) = (u64::from((width >> m).max(1)), u64::from((height >> m).max(1)));
            if fi.compressed {
                w.div_ceil(4) * h.div_ceil(4) * u64::from(fi.block)
            } else {
                w * h * u64::from(fi.block)
            }
        })
        .sum()
}

/// Human-readable bytes.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Where the file lives, as `preview.texture` answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    None,
    Project,
    Library,
    GameFolder,
    GameArchive,
}

impl Origin {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Origin::None => "",
            Origin::Project => "project",
            Origin::Library => "library",
            Origin::GameFolder => "game_folder",
            Origin::GameArchive => "game_archive",
        }
    }
}

/// A texture, loaded and on the GPU.
pub struct TexFile {
    pub name: String,
    /// On disk; empty for a texture inside the game's archives.
    pub file: Option<PathBuf>,
    /// Its gamedata path when it is inside an archive.
    pub archive: String,
    pub origin: Origin,
    pub error: String,
    pub container: &'static str,
    /// The GPU could not take the file as it is: mip 0 decoded to B8G8R8A8.
    pub decoded: bool,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub mips: u32,
    pub dxgi: u32,
    pub texture: Option<wgpu::Texture>,
    /// The file, for the views that read it again (the alpha as grey).
    pub raw: Arc<[u8]>,
    /// The decoded top level, when the file was decoded.
    pub image: Option<Image>,
    /// A texture asset's name in the game, and its keys as written (`use`, `format`, `mips`, ...).
    pub asset: Option<AssetFacts>,
}

/// What a texture asset says of itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssetFacts {
    /// The name the game knows it by (`bunker\wall_brick`).
    pub published: String,
    /// Its source picture.
    pub source: Option<PathBuf>,
    /// Each top-level key of the file but its header, with its value as text.
    pub keys: Vec<(String, String)>,
}

/// A Lua data value as one line of text: a list or a table in braces.
#[must_use]
pub fn value_text(v: &gz_luadata::Value) -> String {
    if v.is_table() {
        let mut parts: Vec<String> = v.arr().iter().map(value_text).collect();
        parts.extend(v.fields().iter().map(|(k, x)| format!("{k} = {}", value_text(x))));
        return format!("{{ {} }}", parts.join(", "));
    }
    v.to_text("")
}

/// Whether a name is a texture asset's project path.
#[must_use]
pub fn is_texture_asset(name: &str) -> bool {
    gz_project::asset_kind::MediaKind::of_file(name) == Some(gz_project::asset_kind::MediaKind::Texture)
}

/// A texture asset of the project baked the way the module build bakes it: the `.dds`, and what
/// the asset says. Blocking.
fn bake_asset(sources: &Sources, rel: &str) -> Result<(Vec<u8>, AssetFacts), String> {
    let root = sources.project.as_ref().ok_or_else(|| "no project is open".to_owned())?;
    let rel = rel.replace('\\', "/");
    let published = sources
        .media
        .as_ref()
        .and_then(|m| m.latest())
        .and_then(|i| i.at(&rel).map(|e| e.name.clone()))
        .unwrap_or_else(|| gz_media::files::published_name(None, &rel, &gz_media::files::module_id(root)));
    let t = gz_media::resolve::texture(root, &rel, &published).map_err(|f| f.to_string())?;
    let made = gz_fmt_tex::make_game_texture(&t.image, &t.params, None).map_err(|e| e.to_string())?;
    let text = std::fs::read(root.join(&rel)).map_err(|e| gz_base::io_reason(&e))?;
    let value = gz_media::kind::load(
        gz_project::asset_kind::MediaKind::Texture,
        &rel,
        &String::from_utf8_lossy(&text),
    )?;
    let keys = value
        .fields()
        .iter()
        .filter(|(k, _)| k.as_str() != "tex")
        .map(|(k, v)| (k.to_string(), value_text(v)))
        .collect();
    Ok((made.dds, AssetFacts { published, source: Some(t.source), keys }))
}

impl TexFile {
    fn failed(name: &str, error: &str) -> Self {
        Self {
            name: name.to_owned(),
            file: None,
            archive: String::new(),
            origin: Origin::None,
            error: error.to_owned(),
            container: "",
            decoded: false,
            file_size: 0,
            width: 0,
            height: 0,
            mips: 0,
            dxgi: 0,
            texture: None,
            raw: Arc::from(Vec::new()),
            image: None,
            asset: None,
        }
    }

    #[must_use]
    pub fn format(&self) -> Option<FormatInfo> {
        dxgi(self.dxgi).map(|(f, _)| f)
    }

    /// The format's text: a file the GPU could not take says what it was decoded from.
    #[must_use]
    pub fn format_text(&self) -> String {
        let gpu = self.format().map_or_else(|| format!("DXGI format {}", self.dxgi), |f| f.name.to_owned());
        if self.decoded {
            format!("{}, decoded to {gpu}", if self.container.is_empty() { "image" } else { self.container })
        } else {
            gpu
        }
    }

    #[must_use]
    pub fn mip_size(&self, mip: u32) -> (u32, u32) {
        ((self.width >> mip).max(1), (self.height >> mip).max(1))
    }
}

/// The bytes of a texture of a source, and where they came from.
fn read(
    sources: &Sources,
    name: &str,
    source: TexSource,
) -> Option<(Vec<u8>, Option<PathBuf>, String, Origin)> {
    match source {
        TexSource::Game => {
            let files = sources.game.as_ref()?;
            let f = files.get(name)?;
            let bytes = files.read_ref(f).ok()?;
            match f.source {
                Source::Loose => Some((
                    bytes,
                    Some(files.game_data().join(f.path.replace('/', "\\"))),
                    String::new(),
                    Origin::GameFolder,
                )),
                Source::Archive { .. } => Some((bytes, None, f.path.to_owned(), Origin::GameArchive)),
            }
        }
        TexSource::Project => {
            let root = sources.project.as_ref()?;
            let path = root.join(name);
            std::fs::read(&path).ok().map(|b| (b, Some(path), String::new(), Origin::Project))
        }
        TexSource::Editor => {
            // a name without an extension, resolved through the library's folders
            let stem = crate::paths::strip_ext(name);
            let probes = [
                (&sources.dirs.game_textures, "dds"),
                (&sources.dirs.textures, "tga"),
                (&sources.dirs.textures, "dds"),
            ];
            probes.into_iter().find_map(|(dir, ext)| {
                let path = dir.join(format!("{stem}.{ext}"));
                if !path.is_file() {
                    return None;
                }
                std::fs::read(&path).ok().map(|b| (b, Some(path), String::new(), Origin::Library))
            })
        }
    }
}

/// What the tab says of a file that is not there: and where it looked.
#[must_use]
pub fn not_found(sources: &Sources, name: &str, source: TexSource) -> String {
    let editor_root = sources.dirs.root.clone();
    let place =
        |dir: &std::path::Path| crate::paths::folder_text(sources.project.as_deref(), &editor_root, dir);
    match source {
        TexSource::Game if sources.game.is_none() => "The file was not found: no game is linked.".to_owned(),
        TexSource::Game => format!("The file was not found. Looked in the linked game's files for {name}."),
        TexSource::Project => match &sources.project {
            Some(root) => {
                format!("The file was not found. Looked in {}.", place(&root.join(name)))
            }
            None => "The file was not found: no project is open.".to_owned(),
        },
        TexSource::Editor => format!(
            "The file was not found. Looked in {} and {}.",
            place(&sources.dirs.game_textures),
            place(&sources.dirs.textures)
        ),
    }
}

/// Reads, decodes when it must, and uploads a texture: on the pools, never a frame.
pub async fn load(editor: Arc<Editor>, name: String, source: TexSource) -> TexFile {
    let Some(gpu) = editor.services.get::<gz_app::WindowLink>().and_then(|l| l.gpu()) else {
        return TexFile::failed(&name, "The editor runs without a graphics device.");
    };
    let sources = Sources::of(&editor).await;
    let n = name.clone();
    let looked = sources.clone();
    let mut facts = None;
    let got = if source == TexSource::Project && is_texture_asset(&name) {
        let made = editor
            .exec
            .io(move || {
                let file = sources.project.as_ref().map(|r| r.join(&n));
                bake_asset(&sources, &n).map(|(dds, f)| (dds, file, f))
            })
            .await;
        match made {
            Ok((dds, file, f)) => {
                facts = Some(f);
                Some((dds, file, String::new(), Origin::Project))
            }
            Err(why) => return TexFile::failed(&name, &why),
        }
    } else {
        editor.exec.io(move || read(&sources, &n, source)).await
    };
    let Some((bytes, file, archive, origin)) = got else {
        return TexFile::failed(&name, &not_found(&looked, &name, source));
    };
    let n = name.clone();
    editor
        .exec
        .cpu(move || {
            let size = bytes.len() as u64;
            let container =
                container_of(&bytes, file.as_ref().map_or(n.as_str(), |f| f.to_str().unwrap_or(&n)));
            let mut t = TexFile::failed(&n, "");
            t.asset = facts;
            t.file = file;
            t.archive = archive;
            t.origin = origin;
            t.container = container;
            t.file_size = size;
            let bc = gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
            if let Some(native) =
                native_dds(&bytes).filter(|d| bc || dxgi(d.dxgi).is_some_and(|(f, _)| !f.compressed))
                && let Some((_, format)) = dxgi(native.dxgi)
                && let Ok(tex) = crate::gpu::upload(
                    &gpu.device,
                    &gpu.queue,
                    "gz-ui-previews texture",
                    format,
                    native.width,
                    native.height,
                    &native.levels,
                )
            {
                t.width = native.width;
                t.height = native.height;
                t.mips = u32::try_from(native.levels.len()).unwrap_or(1);
                t.dxgi = native.dxgi;
                t.texture = Some(tex);
                t.raw = Arc::from(bytes);
                return t;
            }
            let ext = crate::paths::leaf(&n).rsplit_once('.').map(|(_, e)| e.to_lowercase());
            let loaded = match gz_fmt_tex::load_image(&bytes, ext.as_deref()) {
                Ok(l) => l,
                Err(_) => {
                    t.error = UNREADABLE.to_owned();
                    return t;
                }
            };
            let img = loaded.image;
            let mut bgra = img.rgba.clone();
            for px in bgra.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            match crate::gpu::upload(
                &gpu.device,
                &gpu.queue,
                "gz-ui-previews texture",
                wgpu::TextureFormat::Bgra8Unorm,
                img.width,
                img.height,
                &[bgra],
            ) {
                Ok(tex) => {
                    t.width = img.width;
                    t.height = img.height;
                    t.mips = 1;
                    t.dxgi = DECODED;
                    t.decoded = true;
                    t.texture = Some(tex);
                    t.image = Some(img);
                    t.raw = Arc::from(bytes);
                }
                Err(e) => t.error = e,
            }
            t
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(fourcc: &[u8; 4], w: u32, h: u32, mips: u32) -> Vec<u8> {
        let mut p = vec![0u8; 128];
        p[..4].copy_from_slice(b"DDS ");
        p[4..8].copy_from_slice(&124u32.to_le_bytes());
        p[8..12].copy_from_slice(&0x2_0000u32.to_le_bytes());
        p[12..16].copy_from_slice(&h.to_le_bytes());
        p[16..20].copy_from_slice(&w.to_le_bytes());
        p[28..32].copy_from_slice(&mips.to_le_bytes());
        p[80..84].copy_from_slice(&4u32.to_le_bytes());
        p[84..88].copy_from_slice(fourcc);
        p
    }

    #[test]
    fn block_files_are_taken_as_they_are() {
        let mut p = header(b"DXT1", 8, 8, 4);
        // 8x8: 4 blocks, 4x4: 1, 2x2: 1, 1x1: 1
        p.extend(std::iter::repeat_n(0u8, 8 * (4 + 1 + 1 + 1)));
        let d = native_dds(&p).unwrap();
        assert_eq!(d.dxgi, 71);
        assert_eq!(d.levels.len(), 4);
        // cut short: the whole levels stay
        p.truncate(128 + 8 * 4 + 4);
        assert_eq!(native_dds(&p).unwrap().levels.len(), 1);
        // not whole blocks: the decoder's
        let mut q = header(b"DXT5", 6, 8, 1);
        q.extend(std::iter::repeat_n(0u8, 64));
        assert!(native_dds(&q).is_none());
    }

    #[test]
    fn containers_and_sizes() {
        assert_eq!(container_of(b"DDS xxxx", "a"), "DDS");
        assert_eq!(container_of(b"\x89PNG\r\n\x1a\n", "a"), "PNG");
        assert_eq!(container_of(b"xx", "a.TGA"), "TGA");
        let f = dxgi(71).unwrap().0;
        assert_eq!(gpu_bytes(&f, 8, 8, 4), 8 * 4 + 8 + 8 + 8);
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(TexSource::parse("game"), Some(TexSource::Game));
        assert_eq!(TexSource::parse("darf"), None);
    }
}
