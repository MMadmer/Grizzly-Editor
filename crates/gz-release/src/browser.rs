//! The Mod Browser's picture and tags (MOD_CATALOG 6): `[module] thumbnail` - a 256 x 256
//! DXT1/DXT5 DDS the editor makes as `thumbnail.dds` at the module root from any picture - and
//! `[module] tags` from the fixed vocabulary. `mod.thumbnail` makes the picture; the tags are the `tags` key of
//! `mod.set`, plugged into `gz_mod::ManifestArgs` by [`crate::install`].

use std::path::Path;

use futures::FutureExt;
use gz_editor::Editor;
use gz_mod::text::split_list;
use gz_mod::{ArgCx, Edit, ManifestArgs};

/// MOD_CATALOG 6.2, in its order.
pub const TAG_VOCABULARY: &[&str] = &[
    "story",
    "quests",
    "gameplay",
    "weapons",
    "equipment",
    "npcs",
    "mutants",
    "locations",
    "graphics",
    "audio",
    "interface",
    "fixes",
];

/// The thumbnail the editor makes, at the project root.
pub const THUMBNAIL_NAME: &str = "thumbnail.dds";

#[must_use]
pub fn known_tag(tag: &str) -> bool {
    TAG_VOCABULARY.contains(&tag)
}

/// A tag's English label for the editor's UI.
#[must_use]
pub fn tag_label(tag: &str) -> &str {
    match tag {
        "story" => "Story",
        "quests" => "Quests",
        "gameplay" => "Gameplay",
        "weapons" => "Weapons",
        "equipment" => "Equipment",
        "npcs" => "Characters",
        "mutants" => "Mutants",
        "locations" => "Locations",
        "graphics" => "Graphics",
        "audio" => "Sound",
        "interface" => "Interface",
        "fixes" => "Fixes",
        other => other,
    }
}

/// A comma list into the tags `[module] tags` holds: lower case, each once, in
/// the order given; a word outside the vocabulary refuses the whole list.
pub fn parse_tags(value: &str) -> Result<Vec<String>, String> {
    let mut unique: Vec<String> = Vec::new();
    for tag in split_list(value, ',') {
        let tag = tag.to_ascii_lowercase();
        if !known_tag(&tag) {
            return Err(format!("unknown tag '{tag}' (allowed: {})", TAG_VOCABULARY.join(", ")));
        }
        if !unique.contains(&tag) {
            unique.push(tag);
        }
    }
    Ok(unique)
}

/// The edit `[module] tags` gets: the line goes when no tag is left.
pub fn tags_edit(value: &str) -> Result<Edit, String> {
    Ok(Edit::optional("module", "tags", &parse_tags(value)?.join(", ")))
}

/// Bytes are a thumbnail the game's Mod Browser takes (MOD_CATALOG 6.1).
#[must_use]
pub fn valid_thumbnail(bytes: &[u8]) -> bool {
    gz_fmt_tex::valid_thumbnail_dds(bytes)
}

/// The largest file read as a thumbnail or a picture to make one of.
const THUMBNAIL_READ_LIMIT: u64 = 1 << 20;
const PICTURE_READ_LIMIT: u64 = 512 << 20;

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > limit {
        return None;
    }
    std::fs::read(path).ok()
}

/// Any picture (PNG, JPG, TGA, BMP) to the thumbnail the game takes: the
/// middle square scaled to 256, DXT1 (DXT5 with transparency), every level.
pub fn make_thumbnail(picture: &Path) -> Result<Vec<u8>, String> {
    let unreadable = || format!("can't read the picture {} (PNG, JPG, TGA or BMP)", picture.display());
    let bytes = read_limited(picture, PICTURE_READ_LIMIT).ok_or_else(unreadable)?;
    let ext = picture.extension().and_then(|e| e.to_str());
    let loaded = gz_fmt_tex::load_image(&bytes, ext).map_err(|_| unreadable())?;
    let image = loaded.image;
    if image.width < 16 || image.height < 16 {
        return Err(unreadable());
    }
    gz_fmt_tex::make_thumbnail_dds(&image.rgba, image.width, image.height).map_err(|e| match e {
        gz_fmt_tex::TexError::Invalid(text) if text.starts_with("the compressor wrote") => text,
        _ => "the DXT compressor failed".to_owned(),
    })
}

fn is_own_thumbnail(thumbnail: &str) -> bool {
    thumbnail.eq_ignore_ascii_case(THUMBNAIL_NAME)
}

/// What `mod.thumbnail` makes `[module] thumbnail`:
///
/// - empty: no thumbnail; the editor's own `thumbnail.dds` goes with it (one the author pointed
///   at elsewhere in the module is theirs and stays);
/// - a `.dds` inside the project: named as it is, when it is a valid thumbnail;
/// - any other picture: made into `thumbnail.dds` at the root, by way of the editor's scratch.
///
/// Blocking file work: call it on the IO pool.
pub fn apply_thumbnail(project_root: &Path, value: &str, current: &str) -> Result<String, String> {
    let v = gz_mod::text::trim(value).replace('/', "\\");
    let root = project_root.to_string_lossy().into_owned();
    if v.is_empty() {
        if is_own_thumbnail(current) {
            let path = project_root.join(THUMBNAIL_NAME);
            clear_readonly(&path);
            let _ = std::fs::remove_file(&path);
        }
        return Ok(String::new());
    }
    let b = v.as_bytes();
    let absolute = b.len() > 2 && (b.get(1) == Some(&b':') || v.starts_with("\\\\"));
    let full = if absolute { v.clone() } else { format!("{root}\\{v}") };
    let dds =
        Path::new(&full).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dds"));
    if dds {
        // a ready thumbnail inside the project is named as it is
        let n = root.len();
        let inside = full.len() > n + 1
            && full.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(&root))
            && full.as_bytes().get(n) == Some(&b'\\');
        if !inside {
            return Err(format!("a .dds thumbnail has to be a file of the project: {v}"));
        }
        if !read_limited(Path::new(&full), THUMBNAIL_READ_LIMIT).is_some_and(|bytes| valid_thumbnail(&bytes))
        {
            return Err(format!("{v} is not a 256 x 256 DXT1/DXT5 DDS (one mip level or all nine)"));
        }
        return Ok(full.get(n + 1..).unwrap_or("").to_owned());
    }
    // a picture on its way into the project waits in the editor's scratch
    let temp = project_root.join("temp");
    let _ = std::fs::create_dir_all(&temp);
    let pending = temp.join("thumbnail_pending.dds");
    let made = make_thumbnail(Path::new(&full))?;
    let _ = std::fs::remove_file(&pending);
    let target = project_root.join(THUMBNAIL_NAME);
    let cant_write = || format!("can't write {}", target.display());
    std::fs::write(&pending, &made).map_err(|_| cant_write())?;
    clear_readonly(&target);
    if std::fs::rename(&pending, &target).is_err() {
        // another volume: copy, then the scratch copy goes
        std::fs::copy(&pending, &target).map_err(|_| cant_write())?;
        let _ = std::fs::remove_file(&pending);
    }
    Ok(THUMBNAIL_NAME.to_owned())
}

fn clear_readonly(path: &Path) {
    if let Ok(md) = std::fs::metadata(path) {
        let mut perms = md.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

/// Plugs `tags` into `mod.set`.
pub fn install_manifest_args(editor: &Editor) {
    let args = ManifestArgs::of(editor);
    args.provide("tags", |cx: ArgCx| async move { tags_edit(&cx.value).map(|e| vec![e]) }.boxed());
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(parse_tags(" Story, quests ,story,, ").unwrap(), ["story", "quests"]);
        assert!(parse_tags("").unwrap().is_empty());
        assert_eq!(
            parse_tags("story, cars").unwrap_err(),
            "unknown tag 'cars' (allowed: story, quests, gameplay, weapons, equipment, npcs, mutants, locations, graphics, audio, interface, fixes)"
        );
        assert_eq!(tags_edit("").unwrap(), Edit::remove("module", "tags"));
        assert_eq!(tags_edit("npcs,FIXES").unwrap(), Edit::set("module", "tags", "npcs, fixes"));
        assert_eq!(tag_label("npcs"), "Characters");
        assert_eq!(tag_label("odd"), "odd");
    }

    #[test]
    fn thumbnails() {
        let root = std::env::temp_dir().join(format!("gz-release-thumb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // a picture becomes thumbnail.dds at the root
        let picture = root.join("pic.tga");
        let (w, h) = (40u32, 20u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 40, 200, 255]).collect();
        let img = gz_fmt_tex::Image::new(w, h, rgba).unwrap();
        std::fs::write(&picture, gz_fmt_tex::write_tga(&img, gz_fmt_tex::TgaDepth::Bgra32).unwrap()).unwrap();
        assert_eq!(apply_thumbnail(&root, "pic.tga", "").unwrap(), "thumbnail.dds");
        let made = std::fs::read(root.join("thumbnail.dds")).unwrap();
        assert!(valid_thumbnail(&made));
        assert!(!root.join("temp").join("thumbnail_pending.dds").exists());
        // a ready one inside the project is named as it is
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("ui").join("t.dds"), &made).unwrap();
        assert_eq!(apply_thumbnail(&root, "ui/t.dds", "").unwrap(), "ui\\t.dds");
        std::fs::write(root.join("ui").join("bad.dds"), b"DDS nothing").unwrap();
        assert_eq!(
            apply_thumbnail(&root, "ui/bad.dds", "").unwrap_err(),
            "ui\\bad.dds is not a 256 x 256 DXT1/DXT5 DDS (one mip level or all nine)"
        );
        assert!(
            apply_thumbnail(&root, "C:\\elsewhere\\t.dds", "")
                .unwrap_err()
                .starts_with("a .dds thumbnail has to be a file of the project")
        );
        assert!(apply_thumbnail(&root, "missing.png", "").unwrap_err().starts_with("can't read the picture"));
        // removed: the editor's own file goes, the author's stays
        assert_eq!(apply_thumbnail(&root, "", "ui\\t.dds").unwrap(), "");
        assert!(root.join("ui").join("t.dds").exists());
        assert_eq!(apply_thumbnail(&root, " ", "thumbnail.dds").unwrap(), "");
        assert!(!root.join("thumbnail.dds").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
