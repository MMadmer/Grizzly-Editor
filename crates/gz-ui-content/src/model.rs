//! The browser's vocabulary: the two sources, the Library's kinds, locations, hosts, and the
//! small pure rules beside them.

use gz_ui::Icon;

/// Where the browser lists from. The wire names them `project` and `library`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Source {
    /// The open project's own files: the only writable one.
    #[default]
    Project,
    /// What can be brought into the project or placed: the linked game and the editor's library
    /// merged, by kind ([`KINDS`]). Read only.
    Library,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Project, Source::Library];

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Source::Project => 0,
            Source::Library => 1,
        }
    }

    /// `project` or `library` on the wire.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Source::Project => "project",
            Source::Library => "library",
        }
    }

    /// What the author reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Source::Project => "Project",
            Source::Library => "Library",
        }
    }

    /// `project` or `library` (any case).
    #[must_use]
    pub fn parse_name(name: &str) -> Option<Self> {
        Source::ALL.into_iter().find(|s| s.wire().eq_ignore_ascii_case(name))
    }

    #[must_use]
    pub fn read_only(self) -> bool {
        self != Source::Project
    }
}

/// One kind of the Library: a top-level folder of it, with what its entries are.
#[derive(Clone, Copy, Debug)]
pub struct LibKind {
    /// What the author reads: the folder's name.
    pub caption: &'static str,
    /// The wire word (`content.view kind=`).
    pub word: &'static str,
    pub icon: Icon,
    /// What an entry is when dragged: the kit payload's `kind`, which is the `scene.place` kind
    /// for the kinds that are placed.
    pub payload: &'static str,
    /// It is placed in a level (Place in Level, a drop on the viewport).
    pub placed: bool,
    /// The extension of its files (with the dot); `""` for entries without a file of their own.
    pub ext: &'static str,
    /// Where the editor's library keeps them, below its root; `""` for none.
    pub library_folder: &'static str,
    /// Where the game keeps them, `gamedata`-relative; `""` when the game holds none of them.
    pub game_folder: &'static str,
}

/// The Library's kinds, in the order the browser lists them.
pub const KINDS: [LibKind; 7] = [
    LibKind {
        caption: "Objects",
        word: "objects",
        icon: Icon::SceneObject,
        payload: "object",
        placed: true,
        ext: ".object",
        library_folder: "rawdata\\objects",
        game_folder: "",
    },
    LibKind {
        caption: "Groups",
        word: "groups",
        icon: Icon::Group,
        payload: "group",
        placed: true,
        ext: ".group",
        library_folder: "rawdata\\groups",
        game_folder: "",
    },
    LibKind {
        caption: "Textures",
        word: "textures",
        icon: Icon::FileTexture,
        payload: "texture",
        placed: false,
        ext: ".dds",
        library_folder: "gamedata\\textures",
        game_folder: "textures",
    },
    LibKind {
        caption: "Sounds",
        word: "sounds",
        icon: Icon::FileSound,
        payload: "sound",
        placed: true,
        ext: ".ogg",
        library_folder: "gamedata\\sounds",
        game_folder: "sounds",
    },
    LibKind {
        caption: "Particles",
        word: "particles",
        icon: Icon::Particles,
        payload: "particles",
        placed: true,
        ext: "",
        library_folder: "",
        game_folder: "",
    },
    LibKind {
        caption: "Spawn Sections",
        word: "spawn_sections",
        icon: Icon::Spawn,
        payload: "spawn",
        placed: true,
        ext: "",
        library_folder: "",
        game_folder: "",
    },
    LibKind {
        caption: "Base Scenes",
        word: "base_scenes",
        icon: Icon::Level,
        payload: "scene",
        placed: false,
        ext: ".level",
        library_folder: "rawdata\\levels",
        game_folder: "",
    },
];

pub const OBJECTS: usize = 0;
pub const GROUPS: usize = 1;
pub const TEXTURES: usize = 2;
pub const SOUNDS: usize = 3;
pub const PARTICLES: usize = 4;
pub const SPAWNS: usize = 5;
pub const SCENES: usize = 6;
/// The Library's own top: its kinds as folders.
pub const HOME: usize = KINDS.len();

/// A kind of the Library; `None` for [`HOME`] and anything past it.
#[must_use]
pub fn kind(i: usize) -> Option<&'static LibKind> {
    KINDS.get(i)
}

/// A kind by its wire word or its caption, any case; `home` (or nothing) is the Library's top.
#[must_use]
pub fn find_kind(name: &str) -> Option<usize> {
    let n = name.trim();
    if n.is_empty() || n.eq_ignore_ascii_case("home") {
        return Some(HOME);
    }
    let w = n.replace([' ', '-'], "_");
    KINDS.iter().position(|k| k.word.eq_ignore_ascii_case(&w) || k.caption.eq_ignore_ascii_case(n))
}

/// The wire word of a kind index (`home` for the top).
#[must_use]
pub fn kind_word(i: usize) -> &'static str {
    kind(i).map_or("home", |k| k.word)
}

/// Where the browser is: a source, the Library's kind ([`HOME`] at its top; 0 for the project),
/// a folder relative to the kind's (or the project's) root, backslashes, `""` for the root.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Location {
    pub source: Source,
    pub category: usize,
    pub folder: String,
}

impl Location {
    #[must_use]
    pub fn new(source: Source, category: usize, folder: &str) -> Self {
        let category = if source == Source::Project { 0 } else { category.min(HOME) };
        let folder = if source == Source::Library && category == HOME { "" } else { folder };
        Self { source, category, folder: folder.to_owned() }
    }

    /// The Library's top, which lists its kinds.
    #[must_use]
    pub fn home(&self) -> bool {
        self.source == Source::Library && self.category >= HOME
    }

    /// The kind shown, when the Library shows one.
    #[must_use]
    pub fn lib_kind(&self) -> Option<&'static LibKind> {
        if self.source == Source::Library { kind(self.category) } else { None }
    }

    /// Two locations are one when they show the same folder; a kind counts only for the Library.
    #[must_use]
    pub fn same(&self, other: &Location) -> bool {
        self.source == other.source
            && (self.source != Source::Library || self.category == other.category)
            && self.folder.eq_ignore_ascii_case(&other.folder)
    }

    /// What the root of the folder shown is called: the kind, the Library, the project.
    #[must_use]
    pub fn root_label(&self) -> &'static str {
        match self.source {
            Source::Project => "Project",
            Source::Library => kind(self.category).map_or("Library", |k| k.caption),
        }
    }

    /// "Library / Textures / ui": where a favorite points.
    #[must_use]
    pub fn label(&self) -> String {
        let mut label = String::from(self.source.label());
        if let Some(k) = self.lib_kind() {
            label.push_str(" / ");
            label.push_str(k.caption);
        }
        for part in self.folder.split('\\').filter(|p| !p.is_empty()) {
            label.push_str(" / ");
            label.push_str(part);
        }
        label
    }
}

/// Normalises a folder argument: `/` and `\` separate, empty parts go, `.` / `..` and the
/// characters no name may hold are refused.
pub fn normalize_folder(input: &str) -> Result<String, &'static str> {
    let mut out = String::with_capacity(input.len());
    for part in input.split(['\\', '/']) {
        if part.is_empty() {
            continue;
        }
        if part.chars().any(|c| ":*?\"<>|".contains(c)) {
            return Err("folder contains an invalid character");
        }
        if part == "." || part == ".." {
            return Err("folder traversal is not allowed");
        }
        if !out.is_empty() {
            out.push('\\');
        }
        out.push_str(part);
    }
    if out.len() >= 520 {
        return Err("folder path is too long");
    }
    Ok(out)
}

/// The folder above `folder`, `""` above a top-level one.
#[must_use]
pub fn parent_of(folder: &str) -> &str {
    folder.rfind('\\').map_or("", |i| &folder[..i])
}

/// The last part of a path.
#[must_use]
pub fn leaf_of(path: &str) -> &str {
    path.rfind('\\').map_or(path, |i| &path[i + 1..])
}

/// `parent` holds `path` (or is it); the root's empty path holds everything.
#[must_use]
pub fn is_ancestor(parent: &str, path: &str) -> bool {
    if parent.is_empty() {
        return true;
    }
    if parent.len() > path.len() || !path.get(..parent.len()).is_some_and(|h| h.eq_ignore_ascii_case(parent))
    {
        return false;
    }
    parent.len() == path.len() || path.as_bytes().get(parent.len()) == Some(&b'\\')
}

/// A name argument made plain: `/` made `\`, doubled `\` collapsed, a trailing `\` cut.
#[must_use]
pub fn normalize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    while out.ends_with('\\') {
        out.pop();
    }
    out
}

/// 59155 -> "59,155".
#[must_use]
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The two hosts of the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Host {
    /// The Content Drawer over any tab.
    Drawer,
    /// The Level tab's docked panel.
    Panel,
}

impl Host {
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Host::Drawer => 0,
            Host::Panel => 1,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Host::Drawer => "drawer",
            Host::Panel => "panel",
        }
    }

    /// The most a fitted tile takes in this host (pt).
    #[must_use]
    pub fn tile_default(self) -> f32 {
        match self {
            Host::Drawer => gz_app::contract::TILE_DEFAULT,
            Host::Panel => gz_app::contract::TILE_DEFAULT_PANEL,
        }
    }

    /// The layout keys its view is kept under.
    #[must_use]
    pub fn keys(self) -> (&'static str, &'static str) {
        match self {
            Host::Drawer => ("content_browser.tile_size", "content_browser.list"),
            Host::Panel => ("content_browser.tile_size_panel", "content_browser.list_panel"),
        }
    }
}

/// Between two tiles, around a tile's square inside it, the grid's own padding above and below
/// (pt).
pub const TILE_GAP: f32 = 8.0;
pub const TILE_PAD: f32 = 4.0;
pub const GRID_PAD_Y: f32 = 4.0;
/// A list row, the picture at its start (pt), the narrowest list column (em).
pub const LIST_ROW: f32 = 22.0;
pub const LIST_PICTURE: f32 = 18.0;
pub const LIST_COLUMN_EM: f32 = 12.0;
/// The history kept each way.
pub const HISTORY_LIMIT: usize = 64;

/// Tiles under this side are the list: a 48 pt tile cuts a folder's name mid-word.
pub const LIST_BELOW: f32 = 64.0;

/// The side a host's tiles are drawn at (pt). `picked` 0 is no size picked: the largest side up
/// to the host's default, in 8 pt steps, at which the grid shows two whole rows - or every row
/// when the folder needs fewer - 48 at least. `entries` and `width` say how many rows the folder
/// needs.
#[must_use]
pub fn shown_tile(picked: f32, grid_height: f32, most: f32, entries: usize, width: f32) -> f32 {
    if picked > 0.0 {
        return picked;
    }
    if grid_height <= 0.0 {
        return most;
    }
    let min = gz_app::contract::TILE_MIN;
    let mut side = most;
    while side > min {
        let per_row = (((width + TILE_GAP) / (side + TILE_GAP)).floor() as usize).max(1);
        let need = if entries == 0 { 2 } else { entries.div_ceil(per_row).min(2) };
        let cell = side + gz_ui::Font::Small.line() * 2.0 + TILE_PAD + TILE_GAP;
        if ((grid_height + TILE_GAP) / cell).floor() as usize >= need {
            return side;
        }
        side -= 8.0;
    }
    min
}

/// The side, in pixels, an image's picture is made for when drawn `px` pixels wide: the power of
/// two at or over it, 32 to 512.
#[must_use]
pub fn picture_side(px: f32) -> u32 {
    let mut want = 32u32;
    while want < 512 && (want as f32) < px {
        want <<= 1;
    }
    want
}

/// Spaces alone are no search.
#[must_use]
pub fn search_active(query: &str) -> bool {
    query.chars().any(|c| c != ' ' && c != '\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contract_names_the_library_kinds() {
        let captions: Vec<&str> = KINDS.iter().map(|k| k.caption).collect();
        assert_eq!(captions, gz_app::contract::CATEGORIES);
    }

    #[test]
    fn folders_normalise() {
        assert_eq!(normalize_folder("a//b\\c/").unwrap(), "a\\b\\c");
        assert_eq!(normalize_folder("").unwrap(), "");
        assert_eq!(normalize_folder("a/../b"), Err("folder traversal is not allowed"));
        assert_eq!(normalize_folder("a:b"), Err("folder contains an invalid character"));
        assert_eq!(normalize_name("a//b\\\\c/"), "a\\b\\c");
    }

    #[test]
    fn locations() {
        let a = Location::new(Source::Library, 2, "x\\Y");
        assert!(a.same(&Location::new(Source::Library, 2, "X\\y")));
        assert!(!a.same(&Location::new(Source::Library, 3, "x\\y")));
        assert!(Location::new(Source::Project, 2, "t").same(&Location::new(Source::Project, 5, "T")));
        assert_eq!(Location::new(Source::Library, TEXTURES, "ui").label(), "Library / Textures / ui");
        assert_eq!(Location::new(Source::Project, 0, "a\\b").label(), "Project / a / b");
        let home = Location::new(Source::Library, HOME, "ignored");
        assert!(home.home() && home.folder.is_empty() && home.root_label() == "Library");
        assert!(is_ancestor("rawdata", "rawdata\\levels"));
        assert!(!is_ancestor("raw", "rawdata"));
        assert!(is_ancestor("", "x"));
        assert_eq!(parent_of("a\\b\\c"), "a\\b");
        assert_eq!(parent_of("a"), "");
    }

    #[test]
    fn numbers_and_sizes() {
        assert_eq!(thousands(59155), "59,155");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(picture_side(10.0), 32);
        assert_eq!(picture_side(96.0), 128);
        assert_eq!(picture_side(5000.0), 512);
        assert_eq!(shown_tile(120.0, 50.0, 96.0, 10, 800.0), 120.0);
        assert_eq!(shown_tile(0.0, 0.0, 96.0, 10, 800.0), 96.0);
        // the Wide strip: under two rows even of the smallest tiles
        assert_eq!(shown_tile(0.0, 150.0, 64.0, 300, 800.0), 48.0);
        assert_eq!(shown_tile(0.0, 400.0, 96.0, 300, 800.0), 96.0);
        // seven folders in a low drawer fit one row: no need to shrink for a second one
        assert_eq!(shown_tile(0.0, 162.0, 96.0, 7, 1200.0), 96.0);
        assert_eq!(find_kind("Spawn Sections"), Some(SPAWNS));
        assert_eq!(find_kind("base_scenes"), Some(SCENES));
        assert_eq!(find_kind(""), Some(HOME));
        assert!(find_kind("visuals").is_none());
        assert!(!search_active("  \t"));
    }
}
