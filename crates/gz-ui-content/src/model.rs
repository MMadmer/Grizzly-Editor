//! The browser's vocabulary: sources, the Editor source's categories, locations, hosts, and the
//! small pure rules the C++ kept beside them.

use gz_ui::Icon;

/// Where the browser lists from. The wire numbers them 0, 1, 2 and names them project, editor,
/// darf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Source {
    /// The open project's own files: the only writable one.
    #[default]
    Project,
    /// The editor's own library, one category at a time.
    Editor,
    /// The linked game install: archives and loose `gamedata` as one tree. Read only.
    Game,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Project, Source::Editor, Source::Game];

    #[must_use]
    pub fn index(self) -> i64 {
        match self {
            Source::Project => 0,
            Source::Editor => 1,
            Source::Game => 2,
        }
    }

    #[must_use]
    pub fn from_index(i: i64) -> Option<Self> {
        match i {
            0 => Some(Source::Project),
            1 => Some(Source::Editor),
            2 => Some(Source::Game),
            _ => None,
        }
    }

    /// `source_name` on the wire.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Source::Project => "project",
            Source::Editor => "editor",
            Source::Game => "darf",
        }
    }

    /// What the author reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Source::Project => "Project",
            Source::Editor => "Editor",
            Source::Game => "Game",
        }
    }

    /// `project`, `editor`, `darf` (any case).
    #[must_use]
    pub fn parse_name(name: &str) -> Option<Self> {
        Source::ALL.into_iter().find(|s| s.wire().eq_ignore_ascii_case(name))
    }

    #[must_use]
    pub fn read_only(self) -> bool {
        self != Source::Project
    }
}

/// A file of the editor's library that backs an Editor entry: a folder of the editor's data root
/// and an extension. A texture brings its `.thm` and its source along.
#[derive(Clone, Copy, Debug)]
pub struct LibFile {
    pub folder: &'static str,
    pub ext: &'static str,
}

/// One category of the Editor source. `folder` is the folder of the editor's data root it lists;
/// `None` for a category that lives inside one shared file.
#[derive(Clone, Copy, Debug)]
pub struct Category {
    pub caption: &'static str,
    pub folder: Option<&'static str>,
    /// The shared file a folderless category lives in.
    pub shared_file: &'static str,
    pub lib: &'static [LibFile],
    /// The glyph of an entry with no extension of its own.
    pub icon: Icon,
}

const LIB_OBJECT: &[LibFile] = &[LibFile { folder: "rawdata\\objects", ext: ".object" }];
const LIB_LEVEL: &[LibFile] = &[LibFile { folder: "rawdata\\levels", ext: ".level" }];
const LIB_GROUP: &[LibFile] = &[LibFile { folder: "rawdata\\groups", ext: ".group" }];
const LIB_VISUAL: &[LibFile] = &[LibFile { folder: "gamedata\\meshes", ext: ".ogf" }, LibFile { folder: "gamedata\\meshes", ext: ".omf" }];
const LIB_TEXTURE: &[LibFile] = &[
    LibFile { folder: "gamedata\\textures", ext: ".dds" },
    LibFile { folder: "rawdata\\textures", ext: ".thm" },
    LibFile { folder: "rawdata\\textures", ext: ".tga" },
];
const LIB_SOUND: &[LibFile] = &[
    LibFile { folder: "gamedata\\sounds", ext: ".ogg" },
    LibFile { folder: "rawdata\\sounds", ext: ".thm" },
    LibFile { folder: "rawdata\\sounds", ext: ".wav" },
];

/// The Editor source's categories, in the C++ order (`kCategories`, `EditorUiContract`'s
/// `CATEGORIES`).
pub const CATEGORIES: [Category; 10] = [
    Category { caption: "Objects", folder: Some("rawdata\\objects"), shared_file: "", lib: LIB_OBJECT, icon: Icon::SceneObject },
    Category { caption: "Levels", folder: Some("rawdata\\levels"), shared_file: "", lib: LIB_LEVEL, icon: Icon::Level },
    Category { caption: "Groups", folder: Some("rawdata\\groups"), shared_file: "", lib: LIB_GROUP, icon: Icon::Group },
    Category { caption: "Visuals", folder: Some("gamedata\\meshes"), shared_file: "", lib: LIB_VISUAL, icon: Icon::FileModel },
    Category { caption: "Textures", folder: Some("gamedata\\textures"), shared_file: "", lib: LIB_TEXTURE, icon: Icon::FileTexture },
    Category { caption: "Textures (raw)", folder: Some("rawdata\\textures"), shared_file: "", lib: LIB_TEXTURE, icon: Icon::FileTexture },
    Category { caption: "Particles", folder: None, shared_file: "gamedata\\particles.xr", lib: &[], icon: Icon::Particles },
    Category { caption: "Sounds", folder: Some("gamedata\\sounds"), shared_file: "", lib: LIB_SOUND, icon: Icon::FileSound },
    Category { caption: "Entities", folder: None, shared_file: "gamedata\\configs\\system.ltx", lib: &[], icon: Icon::Spawn },
    Category { caption: "Light Anims", folder: None, shared_file: "gamedata\\lanims.xr", lib: &[], icon: Icon::Light },
];

/// The Levels category: its entries open as scenes.
pub const LEVELS: usize = 1;
/// The Visuals and Objects categories: their entries are models.
pub const OBJECTS: usize = 0;
pub const VISUALS: usize = 3;

#[must_use]
pub fn category(i: usize) -> &'static Category {
    CATEGORIES.get(i).unwrap_or(&CATEGORIES[0])
}

/// A category by its caption, any case.
#[must_use]
pub fn find_category(name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    CATEGORIES.iter().position(|c| c.caption.eq_ignore_ascii_case(name))
}

/// Where the browser is: a source, the Editor source's category, a folder relative to the
/// source's root (backslashes, `""` for the root).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Location {
    pub source: Source,
    pub category: usize,
    pub folder: String,
}

impl Location {
    #[must_use]
    pub fn new(source: Source, category: usize, folder: &str) -> Self {
        Self { source, category, folder: folder.to_owned() }
    }

    /// Two locations are one when they show the same folder; a category counts only for the
    /// Editor source (`SameLocation`).
    #[must_use]
    pub fn same(&self, other: &Location) -> bool {
        self.source == other.source && (self.source != Source::Editor || self.category == other.category) && self.folder.eq_ignore_ascii_case(&other.folder)
    }

    /// What the source's root is called in the breadcrumb and the tree.
    #[must_use]
    pub fn root_label(&self) -> &'static str {
        match self.source {
            Source::Project => "Project",
            Source::Game => "Game",
            Source::Editor => category(self.category).caption,
        }
    }

    /// "Game / textures / ui": where a favorite points (`FavoriteLabel`).
    #[must_use]
    pub fn label(&self) -> String {
        let mut label = String::from(match self.source {
            Source::Project => "Project",
            Source::Game => "Game",
            Source::Editor => "Editor",
        });
        if self.source == Source::Editor {
            label.push_str(" / ");
            label.push_str(category(self.category).caption);
        }
        for part in self.folder.split('\\').filter(|p| !p.is_empty()) {
            label.push_str(" / ");
            label.push_str(part);
        }
        label
    }
}

/// Normalises a folder argument: `/` and `\` separate, empty parts go, `.` / `..` and the
/// characters no name may hold are refused (`NormalizeFolderPath`).
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
    if parent.len() > path.len() || !path.get(..parent.len()).is_some_and(|h| h.eq_ignore_ascii_case(parent)) {
        return false;
    }
    parent.len() == path.len() || path.as_bytes().get(parent.len()) == Some(&b'\\')
}

/// A name argument as the C++ took it: `/` made `\`, doubled `\` collapsed, a trailing `\` cut.
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
        if i > 0 && (len - i) % 3 == 0 {
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

/// The side a host's tiles are drawn at (pt). `picked` 0 is no size picked: the largest side up
/// to the host's default at which the grid shows two whole rows, in 8 pt steps, 48 at least
/// (`ShownTile`).
#[must_use]
pub fn shown_tile(picked: f32, grid_height: f32, most: f32) -> f32 {
    if picked > 0.0 {
        return picked;
    }
    if grid_height <= 0.0 {
        return most;
    }
    let fit = (grid_height - TILE_GAP) * 0.5 - gz_ui::Font::Small.line() * 2.0 - TILE_PAD;
    ((fit / 8.0).floor() * 8.0).clamp(gz_app::contract::TILE_MIN, most)
}

/// The side, in pixels, an image's picture is made for when drawn `px` pixels wide: the power of
/// two at or over it, 32 to 512 (`GetThumb`).
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
    fn folders_normalise() {
        assert_eq!(normalize_folder("a//b\\c/").unwrap(), "a\\b\\c");
        assert_eq!(normalize_folder("").unwrap(), "");
        assert_eq!(normalize_folder("a/../b"), Err("folder traversal is not allowed"));
        assert_eq!(normalize_folder("a:b"), Err("folder contains an invalid character"));
        assert_eq!(normalize_name("a//b\\\\c/"), "a\\b\\c");
    }

    #[test]
    fn locations() {
        let a = Location::new(Source::Editor, 2, "x\\Y");
        assert!(a.same(&Location::new(Source::Editor, 2, "X\\y")));
        assert!(!a.same(&Location::new(Source::Editor, 3, "x\\y")));
        assert!(Location::new(Source::Game, 2, "t").same(&Location::new(Source::Game, 5, "T")));
        assert_eq!(Location::new(Source::Game, 0, "textures\\ui").label(), "Game / textures / ui");
        assert_eq!(Location::new(Source::Editor, 4, "").label(), "Editor / Textures");
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
        assert_eq!(shown_tile(120.0, 50.0, 96.0), 120.0);
        assert_eq!(shown_tile(0.0, 0.0, 96.0), 96.0);
        // the Wide strip: under two rows even of the smallest tiles
        assert_eq!(shown_tile(0.0, 150.0, 64.0), 48.0);
        assert_eq!(shown_tile(0.0, 400.0, 96.0), 96.0);
        assert!(find_category("textures (RAW)") == Some(5) && find_category("") .is_none());
        assert!(!search_active("  \t"));
    }
}
