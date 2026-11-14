//! Which picture-less glyph a tile draws and which icon each of the editor's own asset kinds
//! gets. The names are what `content.view` answers in `icons` and `glyphs`.

use gz_project::AssetKind;
use gz_ui::Icon;

/// The drawn icon of one of the editor's own kinds.
#[must_use]
pub fn own_icon(kind: AssetKind) -> Icon {
    match kind {
        AssetKind::Quest => Icon::Quest,
        AssetKind::Behaviour => Icon::Behaviour,
        AssetKind::Character => Icon::Character,
        AssetKind::CharacterTemplate => Icon::CharacterTemplate,
        AssetKind::Pda => Icon::PdaInterface,
        // the project file is the project: the Start page's and the caption's, never a tile
        AssetKind::Project => Icon::Home,
    }
}

/// The `icons` answer and the tooltip's last line.
#[must_use]
pub fn own_name(kind: AssetKind) -> &'static str {
    kind.name()
}

const TYPES: &[(&str, Icon)] = &[
    (".dds", Icon::FileTexture),
    (".tga", Icon::FileTexture),
    (".png", Icon::FileTexture),
    (".jpg", Icon::FileTexture),
    (".jpeg", Icon::FileTexture),
    (".bmp", Icon::FileTexture),
    (".ogf", Icon::FileModel),
    (".object", Icon::FileModel),
    (".omf", Icon::FileModel),
    (".ogg", Icon::FileSound),
    (".wav", Icon::FileSound),
    (".script", Icon::FileScript),
    (".lua", Icon::FileScript),
    (".ltx", Icon::FileConfig),
    (".ltxp", Icon::FileConfig),
    (".ini", Icon::FileConfig),
    (".xml", Icon::FileXml),
    (".xmlp", Icon::FileXml),
    (".txt", Icon::FileText),
    (".md", Icon::FileText),
    (".log", Icon::FileText),
    (".seq", Icon::FileText),
    (".zip", Icon::FileArchive),
    (".7z", Icon::FileArchive),
    (".rar", Icon::FileArchive),
    (".level", Icon::Level),
    (".scene", Icon::Level),
    (".group", Icon::Group),
];

/// The extension of the last part of `path`, with its dot.
#[must_use]
pub fn extension(path: &str) -> Option<&str> {
    let leaf = crate::model::leaf_of(path);
    leaf.rfind('.').filter(|&i| i > 0).and_then(|i| leaf.get(i..))
}

/// The glyph a tile draws while it has no picture: a folder, one of the editor's own kinds, the
/// file type; a Library entry without an extension by its kind (`kind_icon`).
#[must_use]
pub fn tile_icon(path: &str, folder: bool, own: Option<AssetKind>, kind_icon: Option<Icon>) -> Icon {
    if folder {
        return Icon::Folder;
    }
    if let Some(kind) = own {
        return own_icon(kind);
    }
    let Some(ext) = extension(path) else {
        return kind_icon.unwrap_or(Icon::File);
    };
    if let Some((_, icon)) = TYPES.iter().find(|(e, _)| e.eq_ignore_ascii_case(ext)) {
        return *icon;
    }
    // the game's archives: .db0 ... .db9, .xdb0 ..., gamedata.dbN
    let lower = ext.to_ascii_lowercase();
    if lower.starts_with(".db") || lower.starts_with(".xdb") {
        return Icon::FileArchive;
    }
    Icon::File
}

/// An image the tile path decodes.
#[must_use]
pub fn is_image(path: &str) -> bool {
    extension(path).is_some_and(|e| {
        [".dds", ".tga", ".png", ".jpg", ".jpeg", ".bmp"].iter().any(|x| x.eq_ignore_ascii_case(e))
    })
}

/// A model whose picture is rendered.
#[must_use]
pub fn is_model(path: &str) -> bool {
    extension(path).is_some_and(|e| e.eq_ignore_ascii_case(".ogf") || e.eq_ignore_ascii_case(".object"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs() {
        assert_eq!(tile_icon("a\\b", true, None, None).name(), "folder");
        assert_eq!(tile_icon("qa\\notes.txt", false, None, None).name(), "file_text");
        assert_eq!(tile_icon("q.nqasset", false, Some(AssetKind::Quest), None).name(), "quest");
        assert_eq!(
            tile_icon("t.npcasset", false, Some(AssetKind::CharacterTemplate), None).name(),
            "character_template"
        );
        assert_eq!(tile_icon("gamedata.db3", false, None, None), Icon::FileArchive);
        assert_eq!(tile_icon("noext", false, None, Some(Icon::Particles)), Icon::Particles);
        assert_eq!(tile_icon("noext", false, None, None), Icon::File);
        assert_eq!(tile_icon(".gitignore", false, None, None), Icon::File);
        assert!(is_image("ui\\x.DDS") && !is_image("x.ogf") && is_model("m.Object"));
        assert_eq!(own_name(AssetKind::Pda), "PDA interface");
    }
}
