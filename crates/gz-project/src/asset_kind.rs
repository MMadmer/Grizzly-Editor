//! The editor's own asset kinds. The extension is the kind; only a character is looked INTO,
//! to tell a template from a character. The UI maps a kind to its icon.

use serde::Serialize;

/// How much of an `.npcasset` [`kind_of`] needs: a template says so in its first line or two.
pub const HEAD_LEN: usize = 1023;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Quest,
    Behaviour,
    Character,
    CharacterTemplate,
    Pda,
    /// The project file (`<Name>.grizzly`): the project itself, never a tile of the Content
    /// Browser.
    Project,
}

impl AssetKind {
    /// What the author calls it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AssetKind::Quest => "quest",
            AssetKind::Behaviour => "behaviour graph",
            AssetKind::Character => "character",
            AssetKind::CharacterTemplate => "character template",
            AssetKind::Pda => "PDA interface",
            AssetKind::Project => "project",
        }
    }

    /// The extension of its file, with the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            AssetKind::Quest => ".nqasset",
            AssetKind::Behaviour => ".behasset",
            AssetKind::Character | AssetKind::CharacterTemplate => ".npcasset",
            AssetKind::Pda => ".pdaasset",
            AssetKind::Project => crate::project::EXTENSION,
        }
    }

    /// The key that names the kind and its format version in the file (`nq = 1`).
    #[must_use]
    pub fn header(self) -> &'static str {
        match self {
            AssetKind::Quest => "nq",
            AssetKind::Behaviour => "beh",
            AssetKind::Character | AssetKind::CharacterTemplate => "npc",
            AssetKind::Pda => "pda",
            AssetKind::Project => crate::project::HEADER,
        }
    }

    /// The format version this editor writes and the newest it reads.
    #[must_use]
    pub fn version(self) -> u32 {
        match self {
            AssetKind::Quest | AssetKind::Behaviour | AssetKind::Pda => 1,
            AssetKind::Character | AssetKind::CharacterTemplate => 1,
            AssetKind::Project => crate::project::VERSION,
        }
    }

    /// The kind of a file: the one of the extension (a character template reads as a character).
    #[must_use]
    pub fn of_extension(name: &str) -> Option<AssetKind> {
        kind_of(name, None)
    }

    /// Every kind with a file of its own, one per extension.
    pub const FILES: [AssetKind; 4] =
        [AssetKind::Quest, AssetKind::Behaviour, AssetKind::Character, AssetKind::Pda];
}

/// The media kinds: a text asset that describes a source file of a common format beside it (a
/// model, a texture, a sound). The module build publishes the description and the data baked
/// from the source under the game's place for the asset's name; the asset where it lies and its
/// source never ship. The extension is the kind; nothing in the file needs to be looked into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Model,
    Texture,
    Sound,
}

impl MediaKind {
    pub const ALL: [MediaKind; 3] = [MediaKind::Model, MediaKind::Texture, MediaKind::Sound];

    /// What the author calls it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            MediaKind::Model => "model",
            MediaKind::Texture => "texture",
            MediaKind::Sound => "sound",
        }
    }

    /// The extension of its file, with the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            MediaKind::Model => ".meshasset",
            MediaKind::Texture => ".texasset",
            MediaKind::Sound => ".sndasset",
        }
    }

    /// The key that names the kind and its format version in the file (`mesh = 1`).
    #[must_use]
    pub fn header(self) -> &'static str {
        match self {
            MediaKind::Model => "mesh",
            MediaKind::Texture => "tex",
            MediaKind::Sound => "snd",
        }
    }

    /// The format version this editor writes and the newest it reads.
    #[must_use]
    pub fn version(self) -> u32 {
        1
    }

    /// The media kind of a file, by its extension (any case).
    #[must_use]
    pub fn of_file(name: &str) -> Option<MediaKind> {
        let ext = name.rfind('.').and_then(|d| name.get(d..))?;
        Self::ALL.into_iter().find(|k| k.extension().eq_ignore_ascii_case(ext))
    }
}

/// The extensions of the editor's own kinds: the one list every caller asks.
pub const OWN_EXTENSIONS: [&str; 7] =
    [".nqasset", ".behasset", ".npcasset", ".pdaasset", ".meshasset", ".texasset", ".sndasset"];

/// The own extension `name` ends in, as written (a slice of `name`, the dot included). A dot in
/// a folder of the path is no extension, and an extension alone is no name.
#[must_use]
pub fn own_extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    let ext = name.get(dot..)?;
    if ext.contains(['\\', '/']) || name[..dot].ends_with(['\\', '/']) {
        return None;
    }
    OWN_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)).then_some(ext)
}

/// Whether [`kind_of`] needs the first [`HEAD_LEN`] bytes of this file.
#[must_use]
pub fn needs_head(name: &str) -> bool {
    name.rfind('.').and_then(|d| name.get(d..)).is_some_and(|e| e.eq_ignore_ascii_case(".npcasset"))
}

/// What the file `name` is. `head` is the start of the file, for an `.npcasset` only (see
/// [`needs_head`]); without it a character reads as a character.
#[must_use]
pub fn kind_of(name: &str, head: Option<&[u8]>) -> Option<AssetKind> {
    let ext = name.rfind('.').and_then(|d| name.get(d..))?;
    let is = |e: &str| ext.eq_ignore_ascii_case(e);
    if is(".nqasset") {
        Some(AssetKind::Quest)
    } else if is(".behasset") {
        Some(AssetKind::Behaviour)
    } else if is(".pdaasset") {
        Some(AssetKind::Pda)
    } else if is(crate::project::EXTENSION) {
        Some(AssetKind::Project)
    } else if is(".npcasset") {
        let head = head.map(|h| h.get(..HEAD_LEN).unwrap_or(h));
        if head.is_some_and(|h| head_says(h, "abstract", "true")) {
            Some(AssetKind::CharacterTemplate)
        } else {
            Some(AssetKind::Character)
        }
    } else {
        None
    }
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `key = "value"` or `key = value`, as the canonical writer spells it and a hand-written file
/// is likely to. ASCII case is ignored.
#[must_use]
pub fn head_says(text: &[u8], key: &str, value: &str) -> bool {
    let key = key.as_bytes();
    let value = value.as_bytes();
    let len = text.len();
    let mut i = 0;
    while i + key.len() < len {
        let starts_word = i == 0 || text.get(i - 1).is_some_and(|&c| !is_word(c));
        if starts_word && text.get(i..i + key.len()).is_some_and(|k| k.eq_ignore_ascii_case(key)) {
            let mut j = i + key.len();
            if !text.get(j).is_some_and(|&c| is_word(c)) {
                while text.get(j).is_some_and(|&c| c == b' ' || c == b'\t') {
                    j += 1;
                }
                if text.get(j) == Some(&b'=') {
                    j += 1;
                    while text.get(j).is_some_and(|&c| c == b' ' || c == b'\t' || c == b'"') {
                        j += 1;
                    }
                    if text.get(j..j + value.len()).is_some_and(|v| v.eq_ignore_ascii_case(value)) {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_extensions() {
        assert_eq!(own_extension("Intro.NQASSET"), Some(".NQASSET"));
        assert_eq!(own_extension("route.v2.behasset"), Some(".behasset"));
        assert_eq!(own_extension(".npcasset"), None);
        assert_eq!(own_extension("a.pdaasset/x"), None);
        assert_eq!(own_extension("dir/.nqasset"), None);
        assert_eq!(own_extension("model.ogf"), None);
        assert_eq!(own_extension("noext"), None);
        assert_eq!(own_extension("walls/Brick.TexAsset"), Some(".TexAsset"));
        assert_eq!(own_extension("wall.meshasset"), Some(".meshasset"));
        assert_eq!(own_extension("hum.sndasset"), Some(".sndasset"));
    }

    #[test]
    fn media_kinds_by_extension() {
        assert_eq!(MediaKind::of_file("a/b/wall.MESHASSET"), Some(MediaKind::Model));
        assert_eq!(MediaKind::of_file("brick.texasset"), Some(MediaKind::Texture));
        assert_eq!(MediaKind::of_file("hum.sndasset"), Some(MediaKind::Sound));
        assert_eq!(MediaKind::of_file("wall.glb"), None);
        assert_eq!(MediaKind::Texture.header(), "tex");
        // a media asset is no kind the content browser tells apart by looking into it
        assert_eq!(kind_of("wall.meshasset", None), None);
    }

    #[test]
    fn kinds_by_name_and_head() {
        assert_eq!(kind_of("q.nqasset", None), Some(AssetKind::Quest));
        assert_eq!(kind_of("b.behasset", None), Some(AssetKind::Behaviour));
        assert_eq!(kind_of("p.pdaasset", None), Some(AssetKind::Pda));
        assert_eq!(kind_of("c.npcasset", None), Some(AssetKind::Character));
        assert_eq!(
            kind_of("c.npcasset", Some(b"return {\n  abstract = true,\n")),
            Some(AssetKind::CharacterTemplate)
        );
        assert_eq!(
            kind_of("c.npcasset", Some(b"return { abstract = \"TRUE\" }")),
            Some(AssetKind::CharacterTemplate)
        );
        assert_eq!(
            kind_of("c.npcasset", Some(b"return { not_abstract = true }")),
            Some(AssetKind::Character)
        );
        assert_eq!(kind_of("c.npcasset", Some(b"return { abstract = false }")), Some(AssetKind::Character));
        assert_eq!(kind_of("x.ltx", None), None);
        assert!(needs_head("A.NpcAsset"));
        assert!(!needs_head("a.nqasset"));
    }

    #[test]
    fn names() {
        assert_eq!(AssetKind::Pda.name(), "PDA interface");
        assert_eq!(AssetKind::CharacterTemplate.extension(), ".npcasset");
    }
}
