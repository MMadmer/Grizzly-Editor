//! The keys of an item asset: one table per level of the file.
//!
//! The reader ([`crate::model`]), the keys `gz-asset` knows, the diff rules and the reference
//! (`docs/assets/ITEM.md`, made by [`crate::doc`]) all read these tables, so a key cannot mean
//! one thing to the reader and another to its reference.

/// What a key holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    /// The format version.
    Header,
    /// `a-z 0-9 _`, not starting with a digit, up to 48 characters.
    Id,
    /// A section name of the game's configs.
    Section,
    /// A section, or a list of them.
    Sections,
    Bool,
    /// A string (Russian), or a table of strings by language.
    Text,
    String,
    /// A picture file beside the asset.
    File,
    Number {
        min: f64,
        max: f64,
    },
    Integer {
        min: i64,
        max: i64,
    },
    /// The icon in one of its three forms.
    Icon,
    /// `{ key = value }`: keys of a section as the game reads them.
    Keys,
    /// A list of key names.
    KeyList,
    /// `{ key = member or list }`.
    Members,
    /// A number, or `{ min, max }`.
    Count,
    /// `{ <community or community_rank> = chance }`.
    Chances,
    /// The use scene: a string or a table.
    Scene,
    /// A string, a number, a boolean or a list of them.
    Value,
    /// A table; the text says what it holds.
    Table(&'static str),
    /// A list; the text says what it holds.
    List(&'static str),
}

impl Ty {
    /// How the reference names the type.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Ty::Header => "integer".into(),
            Ty::Id => "string (`a-z 0-9 _`)".into(),
            Ty::Section => "string (a section)".into(),
            Ty::Sections => "string or list (sections)".into(),
            Ty::Bool => "boolean".into(),
            Ty::Text => "string or `{ lang = text }`".into(),
            Ty::String => "string".into(),
            Ty::File => "string (a picture beside the asset)".into(),
            Ty::Number { min, max } => format!("number, {min} to {max}"),
            Ty::Integer { min, max } => format!("integer, {min} to {max}"),
            Ty::Icon => "table or string".into(),
            Ty::Keys => "`{ key = value }`".into(),
            Ty::KeyList => "list of keys".into(),
            Ty::Members => "`{ key = member or list }`".into(),
            Ty::Count => "number or `{ min, max }`".into(),
            Ty::Chances => "`{ community = chance }`".into(),
            Ty::Scene => "string or table".into(),
            Ty::Value => "string, number, boolean or list".into(),
            Ty::Table(what) | Ty::List(what) => (*what).into(),
        }
    }
}

/// One key of a level of the file.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub key: &'static str,
    pub ty: Ty,
    /// What a key left out means, as the reference says it; empty: nothing.
    pub default: &'static str,
    pub help: &'static str,
}

const fn f(key: &'static str, ty: Ty, default: &'static str, help: &'static str) -> Field {
    Field { key, ty, default, help }
}

/// The keys a level knows.
#[must_use]
pub fn keys(fields: &[Field]) -> Vec<&'static str> {
    fields.iter().map(|f| f.key).collect()
}

/// A level's field of a key.
#[must_use]
pub fn field<'a>(fields: &'a [Field], key: &str) -> Option<&'a Field> {
    fields.iter().find(|f| f.key == key)
}

const CELL: Ty = Ty::Integer { min: 0, max: 4095 };
const CELLS: Ty = Ty::Integer { min: 1, max: 64 };
const CHANCE: Ty = Ty::Number { min: 0.0, max: 1.0 };

pub const TOP: &[Field] = &[
    f("item", Ty::Header, "", "The format version (1). A file of a newer version is refused by name."),
    f(
        "id",
        Ty::Id,
        "the file's name",
        "A new item's id: `a-z 0-9 _`, not starting with a digit, up to 48 characters.",
    ),
    f(
        "section",
        Ty::Section,
        "`<module id>_<id>`",
        "A new item's section in the game: `a-z 0-9 _ . -`, under 64 characters (a save reads it into 64). The author who names it answers for it not meeting another module's.",
    ),
    f(
        "base",
        Ty::Sections,
        "",
        "A new item's parent section, or a list of them, as `[section]:a, b` (a later one wins over an earlier one): a section of the game, of a module loaded before, or of another item of this module.",
    ),
    f(
        "abstract",
        Ty::Bool,
        "false",
        "A new item that is a template: its section and nothing else - no strings, no icon keys, no list entry.",
    ),
    f(
        "edit",
        Ty::Section,
        "",
        "Instead of `id`, `section` and `base`: the section this file changes. It must exist when the module's items are applied; two files of one module that edit one section are applied in the order of their paths.",
    ),
    f(
        "name",
        Ty::Text,
        "the base's (a new item); unchanged (a change)",
        "What the player reads. The game makes the string id `<section>_name` and sets `inv_name` (over what `set` says), in `configs\\text\\<lang>\\<module>_items.xml` for `rus`, `eng` and every language an item names; a language the asset lacks falls back to `rus`. To name a string id that exists, leave `name` out and write the key itself: `set = { inv_name = \"st_wpn_ak74\" }`.",
    ),
    f(
        "short_name",
        Ty::Text,
        "the name's string id",
        "The short name of the inventory cell: `<section>_short`, `inv_name_short`.",
    ),
    f("description", Ty::Text, "the base's; unchanged", "The description: `<section>_descr`, `description`."),
    f(
        "icon",
        Ty::Icon,
        "the base's; unchanged",
        "`{ texture, x, y, w, h }`: a rectangle of a texture in cells of 50 px (`icons_texture`, `inv_grid_*`), `texture` left out is the game's atlas (`icons_texture` goes), `w` and `h` left out keep the section's size; or a picture beside the asset, `\"<picture>\"` or `{ file = \"<picture>\", w, h }`, which the module build bakes into the module's atlas (below).",
    ),
    f(
        "set",
        Ty::Keys,
        "",
        "Keys of the section as the game reads them: `key = value`. A string, a number, a boolean (`true` / `false`), or a list (`{ a, b }` is `a, b`; numbers likewise: a vector).",
    ),
    f("unset", Ty::KeyList, "", "Keys taken away."),
    f(
        "add",
        Ty::Members,
        "",
        "Members added to a comma list (`ammo_class`, `scopes`, `upgrades`): `{ ammo_class = \"ammo_5.45x39_ap\" }` or a list of them.",
    ),
    f("drop", Ty::Members, "", "Members taken out of a comma list, the same way."),
    f(
        "hud",
        Ty::Table("the HUD section (below)"),
        "",
        "The item's HUD section. A new item: `{ base, set, ... }` makes `<section>_hud` - refused when that section exists - and sets `hud` to it (unless `set.hud` names another). A change (no `base`): `{ set, unset, add, drop }` change the section the item's `hud` key names once `set` is applied.",
    ),
    f(
        "sections",
        Ty::List("a list of sections (below)"),
        "",
        "Other sections the item owns or changes: `{ name, base, set }` makes one (the name as written), `{ edit, set, unset, add, drop }` changes one - sound mixins, scope variants, upgrade sections, a use scene's HUD.",
    ),
    f("craft", Ty::List("a list of recipes (below)"), "", "Recipes that make the item."),
    f(
        "loot",
        Ty::Table("death loot (below)"),
        "",
        "What the dead carry: lines of the game's death loot - the count in `misc\\death_generic.ltx`, the chances in `misc\\death_items_by_communities.ltx`.",
    ),
    f(
        "sold",
        Ty::List("a list of stock rows (below), or one row alone"),
        "",
        "Stock of the game's traders. A module's own character keeps its stock in its `.npcasset`.",
    ),
    f(
        "scene",
        Ty::Scene,
        "",
        "The use animation: `\"vodka\"` uses the scene of that item; a table sets the keys of `items\\items\\animations_settings.ltx` (below), `[<section>]` there over `like`.",
    ),
    f(
        "lists",
        Ty::List("a list of lines (below)"),
        "",
        "Any other line the item needs somewhere: a key of any config, or a member of a list in one.",
    ),
    f("editor", Ty::Table("anything"), "", "The editor's notes. The game reads none of it."),
];

pub const ICON: &[Field] = &[
    f(
        "texture",
        Ty::String,
        "the game's atlas",
        "The texture of the rectangle, by its game name (`ui\\ui_icon_equipment`).",
    ),
    f("x", CELL, "", "The rectangle's left edge, in cells of 50 px (`inv_grid_x`); a rectangle needs it."),
    f("y", CELL, "", "Its top edge, in cells (`inv_grid_y`); a rectangle needs it."),
    f("w", CELLS, "the section's; a picture's: its width in cells", "Its width in cells (`inv_grid_width`)."),
    f(
        "h",
        CELLS,
        "the section's; a picture's: its height in cells",
        "Its height in cells (`inv_grid_height`).",
    ),
    f(
        "file",
        Ty::File,
        "",
        "Instead of a rectangle: a picture beside the asset (`.png`, `.tga`, `.jpg`, `.bmp`, `.dds`), scaled to `w` x `h` cells; left out, they are its size in cells, rounded up.",
    ),
];

pub const HUD: &[Field] = &[
    f(
        "base",
        Ty::Sections,
        "none",
        "A new item's HUD section starts from this one (`wpn_ak74_hud`). Without one it starts from the HUD section its bases name, or from nothing when they name none. A change takes none: it changes the section its `hud` names.",
    ),
    f("set", Ty::Keys, "", "Keys of the HUD section."),
    f("unset", Ty::KeyList, "", "Keys taken away (a change)."),
    f("add", Ty::Members, "", "Members added to a list (a change)."),
    f("drop", Ty::Members, "", "Members taken out of a list (a change)."),
];

pub const SECTION: &[Field] = &[
    f("name", Ty::Section, "", "A section the item makes, named as written."),
    f("base", Ty::Sections, "none", "What the made section starts from."),
    f("set", Ty::Keys, "", "Its keys."),
    f(
        "edit",
        Ty::Section,
        "",
        "Instead of `name`: a section that exists, changed (it keeps its parents: no `base`).",
    ),
    f("unset", Ty::KeyList, "", "Keys taken away (a change)."),
    f("add", Ty::Members, "", "Members added to a list (a change)."),
    f("drop", Ty::Members, "", "Members taken out of a list (a change)."),
];

pub const CRAFT: &[Field] = &[
    f(
        "at",
        Ty::Section,
        "",
        "The tool or the workbench the recipe is in: the section the craft window is opened with (`itm_repairkit_tier_1`).",
    ),
    f("count", Ty::Integer { min: 1, max: 1_000_000 }, "1", "How many the recipe makes."),
    f(
        "parts",
        Ty::List("one to four sections"),
        "",
        "What it takes. The game writes the recipe as `[xms_craft] <n> = at, section, count, part, ...`.",
    ),
];

pub const LOOT: &[Field] = &[
    f(
        "count",
        Ty::Count,
        "",
        "How many the dead carry when they carry it: its line of `[item_count]`, a number or `{ min, max }`. Without one the death manager never spawns the item.",
    ),
    f("chance", CHANCE, "", "Its line of `[base]`, which every community and rank inherits."),
    f(
        "by",
        Ty::Chances,
        "",
        "Lines of their own: `{ bandit = 0.03, stalker_veteran = 0.01 }` - a community, or a community and a rank.",
    ),
];

pub const SOLD: &[Field] = &[
    f(
        "by",
        Ty::String,
        "",
        "The trader's config: `trade_military` (`misc\\trade\\trade_military.ltx`), or a path under `configs` ending in `.ltx`.",
    ),
    f("supplies", Ty::Sections, "\"supplies_1\"", "The section of it, or a list of them."),
    f("count", Ty::Number { min: 0.0, max: 1_000_000.0 }, "1", "How many are on the shelf."),
    f("chance", CHANCE, "1", "The chance they are."),
];

pub const SCENE: &[Field] = &[
    f("like", Ty::Section, "", "The item whose scene this one starts from (`vodka`)."),
    f("sound", Ty::String, "", "The scene's sound (`snd`)."),
    f("camera", Ty::String, "", "The camera effect (`cam`)."),
    f("time", Ty::Number { min: 0.0, max: 600.0 }, "", "How long the scene lasts, seconds (`tm`)."),
    f("helmet", Ty::Value, "", "What a helmet does to it (`helm`), as `animations_settings.ltx` says it."),
    f("anm", Ty::String, "", "The scene's motion."),
    f("hud_anm", Ty::String, "", "The motion of the scene's HUD."),
    f(
        "hud",
        Ty::Table("`{ base, set }`"),
        "",
        "The scene's own HUD section, `item_ea_<section>_hud` of the system config; `anm` left out is then `item_ea_<section>`.",
    ),
];

pub const LIST: &[Field] = &[
    f(
        "file",
        Ty::String,
        "the system config",
        "A config by its path under `configs` (`plugins\\itms_manager.ltx`).",
    ),
    f("section", Ty::Section, "", "The section of it the line goes into."),
    f("key", Ty::String, "the item's section", "The key."),
    f("value", Ty::Value, "a bare key", "`key = value`."),
    f("add", Ty::String, "", "Instead of `value`: a member added to the list `key` holds."),
    f("drop", Ty::String, "", "Instead of `value`: a member taken out of it."),
];

/// Every level of the file: its pattern of `gz-asset`'s known keys and its table.
pub const LEVELS: &[(&str, &[Field])] = &[
    ("", TOP),
    ("icon", ICON),
    ("hud", HUD),
    ("sections.*", SECTION),
    ("craft.*", CRAFT),
    ("loot", LOOT),
    ("sold.*", SOLD),
    ("scene", SCENE),
    ("scene.hud", HUD),
    ("lists.*", LIST),
];
