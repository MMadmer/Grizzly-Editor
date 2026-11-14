//! The one way a file enters the screens: a file of the project
//! stays where it is, one from outside is copied into `<project>\screens`, a PNG / JPG / TGA /
//! BMP becomes a `.dds`, and a `.ps` gets the `.s` the game asks for. THE KIND IS THE FILE'S
//! EXTENSION: nothing is declared.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// What a file is to the screens, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    None,
    Picture,
    Gif,
    Video,
    Shader,
    Page,
    Music,
    /// A picture the game does not read: it becomes a `.dds`.
    Convert,
}

impl FileKind {
    /// What the tab and `screens.info` call a layer of this kind.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Picture => "Picture",
            Self::Gif => "GIF",
            Self::Video => "Video",
            Self::Shader => "Shader",
            Self::Page => "HTML Page",
            _ => "Empty",
        }
    }
}

/// The extension of a path's last component, lower case, with its dot.
#[must_use]
pub fn extension(path: &str) -> String {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    leaf.rfind('.').and_then(|d| leaf.get(d..)).unwrap_or("").to_ascii_lowercase()
}

/// The base name without its extension.
#[must_use]
pub fn stem(path: &str) -> String {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match leaf.rfind('.') {
        Some(dot) if dot > 0 => leaf.get(..dot).unwrap_or(leaf).to_owned(),
        _ => leaf.to_owned(),
    }
}

/// The kind of a file (`FileKind`).
#[must_use]
pub fn file_kind(path: &str) -> FileKind {
    match extension(path).as_str() {
        ".dds" | ".seq" => FileKind::Picture,
        ".gif" => FileKind::Gif,
        ".mp4" | ".webm" | ".wmv" | ".ogm" => FileKind::Video,
        ".s" | ".ps" => FileKind::Shader,
        ".html" | ".htm" => FileKind::Page,
        ".ogg" => FileKind::Music,
        ".png" | ".jpg" | ".jpeg" | ".tga" | ".bmp" => FileKind::Convert,
        _ => FileKind::None,
    }
}

/// `\` separators, no doubled ones, no `\.\`, no trailing one.
#[must_use]
pub fn normalize_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    while let Some(at) = out.find("\\.\\") {
        out.replace_range(at..at + 2, "");
    }
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    out
}

/// `abs` lies below `root` (ASCII case ignored).
#[must_use]
pub fn inside(abs: &str, root: &str) -> bool {
    abs.len() > root.len() + 1
        && abs.get(..root.len()).is_some_and(|h| h.eq_ignore_ascii_case(root))
        && abs.as_bytes().get(root.len()) == Some(&b'\\')
}

/// Where a file given from anywhere ends up, decided by its place and its extension alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The file as given, absolute.
    pub source: String,
    pub kind: FileKind,
    /// It lies outside the project: it is copied into `screens\`.
    pub copy: bool,
    /// It becomes a `.dds`.
    pub convert: bool,
    /// A `.ps` that needs its `.s` beside it.
    pub shader_script: bool,
}

/// A given path made absolute: a relative one is the project's.
fn source_of(root: &str, given: &str) -> String {
    let n = normalize_path(given);
    let absolute = n.len() > 2 && n.as_bytes().get(1) == Some(&b':');
    if absolute { n } else { format!("{root}\\{n}") }
}

/// The plan for a file (before it touches the disk): `Err` with the refusal for a
/// kind the game neither shows nor plays.
pub fn plan(root: &str, given: &str) -> Result<Plan, String> {
    let root = normalize_path(root);
    let source = source_of(&root, given);
    let kind = file_kind(&source);
    if kind == FileKind::None {
        let leaf = source.rsplit('\\').next().unwrap_or(&source);
        return Err(format!(
            "The game shows pictures, GIFs, videos, shaders and HTML pages, and plays .ogg music - not {leaf}."
        ));
    }
    let copy = !inside(&source, &root);
    Ok(Plan {
        convert: kind == FileKind::Convert,
        shader_script: extension(&source) == ".ps",
        copy,
        kind,
        source,
    })
}

/// Creates `dst` from `src` only when nothing is there yet: `false` when something is. Two
/// imports running at once never write over each other.
fn copy_new(src: &Path, dst: &Path) -> io::Result<bool> {
    let mut out = match OpenOptions::new().write(true).create_new(true).open(dst) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut input = File::open(src)?;
    io::copy(&mut input, &mut out)?;
    Ok(true)
}

/// The three lines the game asks a `.ps` for, written for the author when they are missing.
fn ensure_shader_script(ps: &str) -> Result<(), String> {
    let script = format!("{}.s", ps.get(..ps.len().saturating_sub(3)).unwrap_or(ps));
    if Path::new(&script).is_file() {
        return Ok(());
    }
    let name = stem(ps);
    let text = format!(
        "function normal(shader, t_base, t_second, t_detail)\r\n\tshader:begin(\"stub_notransform_t\", \"{name}\")\r\n\t\t: fog(false)\r\n\t\t: zb(false, false)\r\n\t\t: blend(true, blend.srcalpha, blend.invsrcalpha)\r\n\tshader:dx10texture(\"s_base\", t_base)\r\n\tshader:dx10sampler(\"smp_base\")\r\nend\r\n"
    );
    fs::write(&script, text).map_err(|_| format!("can't write {script}"))
}

/// A screen layer's picture as a `.dds`.
fn make_picture(image: &str, dds: &str) -> Result<(), String> {
    let unreadable = || format!("can't read the picture {image} (PNG, JPG, TGA or BMP)");
    let bytes = fs::read(image).map_err(|_| unreadable())?;
    let loaded = gz_fmt_tex::load_image(&bytes, Some(&extension(image))).map_err(|_| unreadable())?;
    let img = loaded.image;
    if img.width < 1 || img.height < 1 {
        return Err(unreadable());
    }
    let out = gz_fmt_tex::make_screen_picture_dds(&img.rgba, img.width, img.height)
        .map_err(|_| "the DXT compressor failed".to_owned())?;
    let _ = fs::remove_file(dds);
    fs::write(dds, out).map_err(|_| "the DXT compressor failed".to_owned())
}

/// Imports a file into the project and answers its project-relative path with `\`.
/// Blocking file work: run it on the IO pool. Several imports may run at once.
pub fn import(root: &str, given: &str) -> Result<String, String> {
    let root = normalize_path(root);
    if !Path::new(&source_of(&root, given)).is_file() {
        return Err(format!("The file does not exist: {given}"));
    }
    let p = plan(&root, given)?;
    let abs = p.source.clone();
    let mut target = abs.clone();
    if p.copy {
        let folder = format!("{root}\\screens");
        let _ = fs::create_dir(&folder);
        let leaf = abs.rsplit('\\').next().unwrap_or(&abs).to_owned();
        let (base, ext) = match leaf.rfind('.') {
            Some(dot) => {
                (leaf.get(..dot).unwrap_or(&leaf).to_owned(), leaf.get(dot..).unwrap_or("").to_owned())
            }
            None => (leaf.clone(), String::new()),
        };
        let source = fs::read(&abs).map_err(|_| format!("The file cannot be read: {abs}"))?;
        let mut n = 1u32;
        target = loop {
            if n > 100_000 {
                return Err(format!("The file cannot be copied into the project: {abs}"));
            }
            let candidate =
                if n == 1 { format!("{folder}\\{leaf}") } else { format!("{folder}\\{base}_{n}{ext}") };
            let path = Path::new(&candidate);
            if path.exists() {
                // the same file there already is the one to use
                if fs::read(path).is_ok_and(|there| there == source) {
                    break candidate;
                }
                n += 1;
                continue;
            }
            match copy_new(Path::new(&abs), path) {
                Ok(true) => break candidate,
                // somebody made it this very moment: look at it again
                Ok(false) => {}
                Err(_) => return Err(format!("The file cannot be copied into the project: {abs}")),
            }
        };
        // a shader is two files, and a .ps may be picked for either
        if p.kind == FileKind::Shader {
            let from = abs.get(..abs.rfind('.').unwrap_or(abs.len())).unwrap_or(&abs).to_owned();
            let to = target.get(..target.rfind('.').unwrap_or(target.len())).unwrap_or(&target).to_owned();
            for e in [".s", ".ps"] {
                let sibling = format!("{from}{e}");
                if Path::new(&sibling).is_file() {
                    let _ = fs::copy(&sibling, format!("{to}{e}"));
                }
            }
        }
    }
    if p.convert {
        let dds =
            format!("{}.dds", target.get(..target.rfind('.').unwrap_or(target.len())).unwrap_or(&target));
        make_picture(&target, &dds)?;
        if p.copy {
            let _ = fs::remove_file(&target);
        }
        target = dds;
    }
    if extension(&target) == ".ps" {
        ensure_shader_script(&target)?;
    }
    Ok(target.get(root.len() + 1..).unwrap_or(&target).to_owned())
}
