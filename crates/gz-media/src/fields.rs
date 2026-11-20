//! The keys of the three media kinds: one table per level of each file.
//!
//! The reader ([`crate::model`], [`crate::texture`], [`crate::sound`]), the check and the
//! references (`docs/assets/MODEL.md`, `TEXTURE.md`, `SOUND.md`, made by [`crate::doc`]) all read
//! these tables, so a key cannot mean one thing to the reader and another to its reference. A
//! word list holds the value a word stands for in the game's own numbers.

/// What a key holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    /// The format version.
    Header,
    Text,
    /// A file beside the asset, relative to its folder.
    File,
    /// A name of the game's namespace (`bunker\wall_brick`).
    GameName,
    Bool,
    Number {
        min: f64,
        max: f64,
    },
    Integer {
        min: i64,
        max: i64,
    },
    /// One word of a list.
    Words(&'static [(&'static str, u32)]),
    /// One word of a list, or a list of them: their values together.
    WordSet(&'static [(&'static str, u32)]),
    /// `{ x, y, z }`.
    Vec3,
    /// `{ x, y, z }` in degrees.
    Angles,
    /// `{ r, g, b, a }`, each 0 to 255.
    Color,
    /// A table; the text says what it holds.
    Table(&'static str),
}

impl Ty {
    /// How the reference names the type.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Ty::Header | Ty::Integer { .. } => "integer".into(),
            Ty::Text => "string".into(),
            Ty::File => "string (a file beside the asset)".into(),
            Ty::GameName => "string (a game name)".into(),
            Ty::Bool => "boolean".into(),
            Ty::Number { .. } => "number".into(),
            Ty::Words(w) => {
                let mut seen: Vec<&str> = Vec::new();
                for (x, _) in *w {
                    if !seen.contains(x) {
                        seen.push(x);
                    }
                }
                seen.iter().map(|x| format!("`\"{x}\"`")).collect::<Vec<_>>().join(" ")
            }
            Ty::WordSet(w) => format!(
                "one of these words, or a list of them: {}",
                w.iter().map(|(x, _)| format!("`\"{x}\"`")).collect::<Vec<_>>().join(" ")
            ),
            Ty::Vec3 => "{ x, y, z }".into(),
            Ty::Angles => "{ x, y, z } (degrees)".into(),
            Ty::Color => "{ r, g, b, a } (0-255)".into(),
            Ty::Table(what) => (*what).into(),
        }
    }
}

/// One key of a level of a file.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub key: &'static str,
    pub ty: Ty,
    /// What a key left out means, as the reference says it; empty: nothing.
    pub default: &'static str,
    pub help: &'static str,
    /// What the window calls the key (Title Case); empty: the window shows no row of it.
    pub title: &'static str,
    /// The window's tip of the row; empty: the help.
    pub tip: &'static str,
    /// The window shows the row where an author meets it, not under More.
    pub surface: bool,
    /// The unit the window writes after a number (`m`); empty: none.
    pub unit: &'static str,
}

const fn f(key: &'static str, ty: Ty, default: &'static str, help: &'static str) -> Field {
    Field { key, ty, default, help, title: "", tip: "", surface: false, unit: "" }
}

impl Field {
    /// The same key with the window's title.
    const fn titled(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    /// The same key with a shorter tip than its help.
    const fn tip(mut self, tip: &'static str) -> Self {
        self.tip = tip;
        self
    }

    /// The same key with the unit of its number.
    const fn unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// The same key on the window's surface.
    const fn on_surface(mut self) -> Self {
        self.surface = true;
        self
    }

    /// The window's tip of the row.
    #[must_use]
    pub fn shown_tip(&self) -> &'static str {
        if self.tip.is_empty() { self.help } else { self.tip }
    }
}

/// The word of a value in a list: the first word that stands for it.
#[must_use]
pub fn word_of(words: &[(&'static str, u32)], value: u32) -> Option<&'static str> {
    words.iter().find(|(_, v)| *v == value).map(|(w, _)| *w)
}

/// The value of a word, any case.
#[must_use]
pub fn value_of(words: &[(&'static str, u32)], word: &str) -> Option<u32> {
    words.iter().find(|(w, _)| w.eq_ignore_ascii_case(word)).map(|(_, v)| *v)
}

/// The keys a level knows, for `KindOps::known` and the reader's checks.
#[must_use]
pub fn keys(fields: &[Field]) -> Vec<&'static str> {
    fields.iter().map(|f| f.key).collect()
}

// ---- the words -----------------------------------------------------------------------------

pub const MODEL_KINDS: &[(&str, u32)] = &[("static", 0), ("dynamic", 1)];

pub const SHAPES: &[(&str, u32)] = &[("box", 0), ("ramp", 1), ("cylinder", 2), ("plane", 3), ("prism", 4)];

/// A bone's collision shape, as the model's `ShapeType` numbers it.
pub const BONE_SHAPES: &[(&str, u32)] = &[("none", 0), ("box", 1), ("sphere", 2), ("cylinder", 3)];

/// The texture types (`TextureType`); `ui` is a 2D texture without mip levels.
pub const USES: &[(&str, u32)] =
    &[("surface", 0), ("ui", 0), ("cube", 1), ("bump", 2), ("normal", 3), ("terrain", 4)];

/// What `format = "auto"` stands for in [`FORMATS`].
pub const AUTO: u32 = u32::MAX;

/// The texture formats (`TextureFormat`).
pub const FORMATS: &[(&str, u32)] = &[
    ("auto", AUTO),
    ("dxt1", 0),
    ("dxt1_alpha", 1),
    ("dxt3", 2),
    ("dxt5", 3),
    ("rgba4444", 4),
    ("rgba1555", 5),
    ("rgb565", 6),
    ("rgb", 7),
    ("rgba", 8),
    ("nvhs", 9),
    ("nvhu", 10),
    ("a8", 11),
    ("l8", 12),
    ("a8l8", 13),
    ("bc4", 14),
    ("bc5", 15),
    ("bc6", 16),
    ("bc7", 17),
];

/// The mip filters (`MipFilterId`).
pub const MIP_FILTERS: &[(&str, u32)] = &[
    ("box", 0),
    ("cubic", 1),
    ("point", 2),
    ("triangle", 3),
    ("quadratic", 4),
    ("advanced", 5),
    ("catrom", 6),
    ("mitchell", 7),
    ("gaussian", 8),
    ("sinc", 9),
    ("bessel", 10),
    ("hanning", 11),
    ("hamming", 12),
    ("blackman", 13),
    ("kaiser", 14),
];

/// The lighting models (`TextureMaterial`): each one blends towards the next by `weight`.
pub const MATERIALS: &[(&str, u32)] = &[("oren_nayar", 0), ("blinn", 1), ("phong", 2), ("metal", 3)];

/// What the detail texture is laid over.
pub const DETAIL_ON: &[(&str, u32)] = &[("none", 0), ("diffuse", 1), ("bump", 2), ("both", 3)];

/// What a bump source holds.
pub const BUMP_FROM: &[(&str, u32)] = &[("height", 0), ("normal", 1)];

/// The game's sound types, by the game's own names (its `SOUND_TYPE_` list without the prefix,
/// in lower case): what NPCs make of a sound. The whole kinds first - a value is named by the
/// first word that stands for it -, then the single bits they are made of.
pub const SOUND_TYPES: &[(&str, u32)] = &[
    ("no_sound", 0),
    ("item_picking_up", 0x4000_0000 | 0x0400_0000),
    ("item_dropping", 0x4000_0000 | 0x0200_0000),
    ("item_hiding", 0x4000_0000 | 0x0100_0000),
    ("item_taking", 0x4000_0000 | 0x0080_0000),
    ("item_using", 0x4000_0000 | 0x0040_0000),
    ("weapon_shooting", 0x8000_0000 | 0x0020_0000),
    ("weapon_empty_clicking", 0x8000_0000 | 0x0010_0000),
    ("weapon_bullet_hit", 0x8000_0000 | 0x0008_0000),
    ("weapon_recharging", 0x8000_0000 | 0x0004_0000),
    ("monster_dying", 0x2000_0000 | 0x0002_0000),
    ("monster_injuring", 0x2000_0000 | 0x0001_0000),
    ("monster_step", 0x2000_0000 | 0x0000_8000),
    ("monster_talking", 0x2000_0000 | 0x0000_4000),
    ("monster_attacking", 0x2000_0000 | 0x0000_2000),
    ("monster_eating", 0x2000_0000 | 0x0000_1000),
    ("anomaly_idle", 0x1000_0000 | 0x0000_0800),
    ("world_object_breaking", 0x0800_0000 | 0x0000_0400),
    ("world_object_colliding", 0x0800_0000 | 0x0000_0200),
    ("world_object_exploding", 0x0800_0000 | 0x0000_0100),
    ("world_ambient", 0x0800_0000 | 0x0000_0080),
    ("weapon", 0x8000_0000),
    ("item", 0x4000_0000),
    ("monster", 0x2000_0000),
    ("anomaly", 0x1000_0000),
    ("world", 0x0800_0000),
    ("picking_up", 0x0400_0000),
    ("dropping", 0x0200_0000),
    ("hiding", 0x0100_0000),
    ("taking", 0x0080_0000),
    ("using", 0x0040_0000),
    ("shooting", 0x0020_0000),
    ("empty_clicking", 0x0010_0000),
    ("bullet_hit", 0x0008_0000),
    ("recharging", 0x0004_0000),
    ("dying", 0x0002_0000),
    ("injuring", 0x0001_0000),
    ("step", 0x0000_8000),
    ("talking", 0x0000_4000),
    ("attacking", 0x0000_2000),
    ("eating", 0x0000_1000),
    ("idle", 0x0000_0800),
    ("object_breaking", 0x0000_0400),
    ("object_colliding", 0x0000_0200),
    ("object_exploding", 0x0000_0100),
    ("ambient", 0x0000_0080),
];

/// A set's value as its file says it: the one word that stands for it, or the words of its
/// bits (a bit no word names is left out).
#[must_use]
pub fn words_of(words: &[(&'static str, u32)], value: u32) -> Vec<&'static str> {
    if let Some(w) = word_of(words, value) {
        return vec![w];
    }
    words.iter().filter(|(_, v)| v.count_ones() == 1 && value & v != 0).map(|(w, _)| *w).collect()
}

// ---- the model -----------------------------------------------------------------------------

const POS: Ty = Ty::Vec3;
const METRES: Ty = Ty::Number { min: 0.001, max: 10_000.0 };

pub const MODEL: &[Field] = &[
    f(
        "mesh",
        Ty::Header,
        "",
        "The header: the format version (1). A file of a newer version is refused by name.",
    ),
    f(
        "source",
        Ty::File,
        "the file with the asset's own name and `.glb`, `.gltf`, `.obj`, `.object` or `.ogf`",
        "The model's geometry: glTF 2.0, Wavefront OBJ, an X-Ray SDK `.object`, or a compiled model - an `.ogf` of the project, or a model of the game by its engine name (`\"game:dynamics/box/box_wood_01.ogf\"`: the project's own `gamedata\\meshes` first, then the linked game). Over a compiled model the asset says what changes: `surfaces` by the name of the texture a part wears, `userdata`, a bone's `mass` and `material`; a dynamic one is published as the same file with that said again - its skeleton, its motions and its levels of detail stay its own -, and as the very file when the asset changes nothing of it. Not together with `parts`.",
    ),
    f(
        "name",
        Ty::GameName,
        "the path below `gamedata\\meshes` for an asset that lies there, else `<module id>\\<file name>`",
        "The name the game knows the model by: a dynamic model is published as `meshes\\<name>.ogf`. A static one is compiled into the level and needs none.",
    ),
    f(
        "kind",
        Ty::Words(MODEL_KINDS),
        "\"static\"",
        "`static`: geometry the level build compiles into the level; `dynamic`: an object the game spawns, published as its own model.",
    ),
    f("scale", Ty::Number { min: 0.001, max: 1000.0 }, "1", "Applied to the source as it is read."),
    f(
        "rotate",
        Ty::Angles,
        "{ 0, 0, 0 }",
        "Turns the source as it is read: heading (Y), then pitch (X), then bank (Z). Not applied to a model with bones.",
    ),
    f(
        "surfaces",
        Ty::Table("a list of surfaces (below)"),
        "the source's",
        "Restates the source's surfaces by their names; a key left out keeps the source's value.",
    ),
    f(
        "parts",
        Ty::Table("a list of parts (below)"),
        "none",
        "Geometry written in the asset instead of a source: a blockout an agent types.",
    ),
    f(
        "bones",
        Ty::Table("a list of bones (below)"),
        "the source's",
        "A dynamic model's bones by name: their physics. A dynamic model without bones gets one bone, `root`.",
    ),
    f(
        "animations",
        Ty::Table("a list of animations (below)"),
        "the source's clips",
        "A dynamic model's motions: the source's clips renamed and set.",
    ),
    f(
        "userdata",
        Ty::Text,
        "none",
        "A dynamic model's user data: the ini text the game reads from the model.",
    ),
];

pub const SURFACE: &[Field] = &[
    f("name", Ty::Text, "", "The source's material name the row is about (any case)."),
    f(
        "texture",
        Ty::GameName,
        "the texture asset of the source's picture",
        "The texture, by its game name (a texture asset's `name`, a texture of the project or the game).",
    ),
    f("shader", Ty::Text, "\"default\"", "The engine shader (`shaders.xr` of the linked game)."),
    f("compile", Ty::Text, "\"default\"", "The level compiler's shader (`shaders_xrlc.xr`)."),
    f(
        "material",
        Ty::Text,
        "\"default\"",
        "The game material (`gamemtl.xr`): how the surface sounds, breaks and is walked on.",
    ),
    f("two_sided", Ty::Bool, "the source's", "Both sides are drawn."),
];

pub const PART: &[Field] = &[
    f(
        "shape",
        Ty::Words(SHAPES),
        "",
        "What the part is: a box, a ramp (a wedge rising along +Z), a cylinder (upright), a plane (facing up) or a prism (an outline extruded up).",
    ),
    f(
        "size",
        Ty::Table("{ x, y, z } metres; a plane's { x, z }"),
        "{ 1, 1, 1 }",
        "The box, the ramp or the plane. A box and a ramp stand on their base: `pos` is the middle of the bottom.",
    ),
    f("pos", POS, "{ 0, 0, 0 }", "Where the part stands, metres."),
    f(
        "rot",
        Ty::Angles,
        "{ 0, 0, 0 }",
        "How the part is turned: heading (Y), then pitch (X), then bank (Z).",
    ),
    f(
        "surface",
        Ty::Text,
        "\"default\"",
        "The surface the part wears: a row of `surfaces`, or one made with the defaults.",
    ),
    f("tile", METRES, "1", "The texture repeats every this many metres (laid by box mapping)."),
    f("radius", METRES, "0.5", "A cylinder's radius."),
    f("height", METRES, "1", "A cylinder's or a prism's height."),
    f("sides", Ty::Integer { min: 3, max: 256 }, "16", "A cylinder's sides."),
    f(
        "outline",
        Ty::Table("a list of { x, z } points, metres"),
        "",
        "A prism's outline seen from above, any simple polygon, either way round.",
    ),
];

pub const BONE: &[Field] = &[
    f("name", Ty::Text, "", "The bone the row is about."),
    f(
        "shape",
        Ty::Words(BONE_SHAPES),
        "\"none\"",
        "The bone's collision shape, fitted around the bone's vertices.",
    ),
    f("mass", Ty::Number { min: 0.0, max: 100_000.0 }, "10", "The bone's mass, kilograms."),
    f(
        "material",
        Ty::Text,
        "\"default_object\"",
        "The bone's game material, a dynamic one of `gamemtl.xr` (`objects\\...`): said to the game in the model's description, which takes it when it is dynamic and keeps the model's own otherwise. The published model itself keeps the material its source has.",
    ),
];

pub const ANIMATION: &[Field] = &[
    f("name", Ty::Text, "", "The motion's name in the game."),
    f("clip", Ty::Text, "the row's `name`", "The source's clip the motion is made of."),
    f("speed", Ty::Number { min: 0.0, max: 100.0 }, "1", "How fast the motion plays."),
    f("loop", Ty::Bool, "true", "The motion starts again at its end; false: it stops there."),
    f("no_mix", Ty::Bool, "false", "The motion replaces what plays instead of blending in."),
    f("sync_part", Ty::Bool, "false", "The motion keeps time with the other parts of the body."),
    f("foot_steps", Ty::Bool, "false", "The motion's marks sound as steps."),
    f("root_mover", Ty::Bool, "false", "The root bone's motion moves the object."),
    f("idle", Ty::Bool, "false", "A motion the game may cut short: an idle."),
    f("weapon_bone", Ty::Bool, "false", "The motion drives the weapon's bone."),
];

// ---- the texture ---------------------------------------------------------------------------

pub const TEXTURE: &[Field] = &[
    f(
        "tex",
        Ty::Header,
        "",
        "The header: the format version (1). A file of a newer version is refused by name.",
    ),
    f(
        "source",
        Ty::File,
        "the file with the asset's own name and `.png`, `.jpg`, `.tga`, `.bmp` or `.dds`",
        "The picture. A `.dds` is a game texture already: it is published as it is - with its `#` companion beside it, `<stem>#.dds`, the second texture of a bump pair -, and the keys that say how a picture is made (`format`, `mips`, the dithers, the borders, the fades, a bump map's `normal_map` and `depth`) apply to nothing - a warning at each. `\"game:<name>\"` names a texture of the game: the asset is its description alone, published under that name over the game's picture (`name` is that texture's).",
    ),
    f(
        "name",
        Ty::GameName,
        "the path below `gamedata\\textures` for an asset that lies there, else `<module id>\\<file name>`",
        "The name the game knows the texture by: it is published as `textures\\<name>.dds`.",
    ),
    f(
        "use",
        Ty::Words(USES),
        "\"surface\"",
        "What the texture is: a surface's picture, a picture of the interface (no mip levels), a cube map, a bump map, a normal map or a terrain's picture. Picking one sets what that use needs; the keys below set the rest. The game reads the description of a surface, a normal map and a terrain's picture; of `ui`, `cube` and `bump` nothing, as of such a `.thm`.",
    ),
    f(
        "format",
        Ty::Words(FORMATS),
        "\"auto\"",
        "How the game texture is stored; `auto`: DXT1, or DXT5 when the picture has alpha (what `use` needs otherwise).",
    ),
    f(
        "mips",
        Ty::Bool,
        "true (false for `ui`, `cube`, `bump`, `terrain`)",
        "The texture keeps its mip levels.",
    ),
    f(
        "mip_filter",
        Ty::Words(MIP_FILTERS),
        "\"box\" (`kaiser` for `normal`)",
        "How the mip levels are made.",
    ),
    f("has_alpha", Ty::Bool, "the picture's", "The picture has an alpha channel."),
    f("binary_alpha", Ty::Bool, "false", "Alpha is either on or off."),
    f("dither", Ty::Bool, "true", "The colours are dithered when they are reduced."),
    f("dither_each_mip", Ty::Bool, "false", "Every mip level is dithered on its own."),
    f(
        "implicit_light",
        Ty::Bool,
        "false (true for `terrain`)",
        "The level build lights the texture by its own alpha.",
    ),
    f("alpha_border", Ty::Bool, "false", "The mip levels get a transparent border."),
    f("color_border", Ty::Bool, "false", "The mip levels get a border of `border_color`."),
    f("border_color", Ty::Color, "{ 0, 0, 0, 0 }", "The border's colour."),
    f("fade_to_color", Ty::Bool, "false", "The smaller mip levels fade to `fade_color`."),
    f("fade_to_alpha", Ty::Bool, "false", "The smaller mip levels fade to transparent."),
    f("fade_color", Ty::Color, "{ 0, 0, 0, 0 }", "The colour the mip levels fade to."),
    f(
        "fade_amount",
        Ty::Integer { min: 0, max: 100 },
        "0",
        "How much of the colour the mip levels fade to, per cent.",
    ),
    f("fade_delay", Ty::Integer { min: 0, max: 255 }, "0", "The mip level the fade starts at."),
    f(
        "material",
        Ty::Words(MATERIALS),
        "\"blinn\"",
        "How the game lights the surface: each model blends towards the next one by `weight` (Oren-Nayar, Blinn, Phong, metal, back to Oren-Nayar).",
    ),
    f(
        "weight",
        Ty::Number { min: 0.0, max: 1.0 },
        "0",
        "How far `material` is blended towards the next model.",
    ),
    f("normal_map", Ty::GameName, "none", "A bump map's own normal map, by its game name."),
    f(
        "bump",
        Ty::Table(
            "the bump map (below), `true` for one under the texture's name with `_bump`, `false` for none",
        ),
        "none",
        "The texture has a bump map: the game draws the surface's relief with it.",
    ),
    f(
        "detail",
        Ty::Table("the detail texture (below), or `false` for none"),
        "none",
        "A finer texture laid over this one close up.",
    ),
    f(
        "extra_flags",
        Ty::Integer { min: 0, max: 4_294_967_295 },
        "0",
        "Flag bits of a texture description no key names, kept as they are.",
    ),
    f(
        "depth",
        Ty::Number { min: 0.0, max: 1.0 },
        "0.05",
        "How deep the relief of a bump map made from a height map is, metres, for a texture that has no `bump` of its own - a bump map itself (`use = \"bump\"`) says it here; a texture with a `bump` says it there.",
    ),
];

pub const BUMP: &[Field] = &[
    f(
        "texture",
        Ty::GameName,
        "`<name>_bump`",
        "The bump map by its game name, when it is a texture of its own. Said empty (`\"\"`), the texture uses a bump map of no name: the game draws none.",
    ),
    f(
        "source",
        Ty::File,
        "none",
        "A picture the bump map is made of: the build publishes it as `textures\\<name>_bump.dds` with its `<name>_bump#.dds`.",
    ),
    f(
        "from",
        Ty::Words(BUMP_FROM),
        "\"height\"",
        "What `source` holds: a height map (gloss in alpha) or a normal map.",
    ),
    f("depth", Ty::Number { min: 0.0, max: 1.0 }, "0.05", "How deep the relief is, metres."),
    f("parallax", Ty::Bool, "false", "The relief is drawn with parallax."),
];

pub const DETAIL: &[Field] = &[
    f(
        "texture",
        Ty::GameName,
        "",
        "The detail texture by its game name. Without it the game lays no detail texture.",
    ),
    f("scale", Ty::Number { min: 0.0, max: 10_000.0 }, "1", "How many times it repeats over this texture."),
    f(
        "on",
        Ty::Words(DETAIL_ON),
        "\"diffuse\"",
        "What it is laid over: the colour, the bump map, both, or neither (`none`: the game lays no detail texture).",
    ),
];

// ---- the sound -----------------------------------------------------------------------------

pub const SOUND: &[Field] = &[
    f(
        "snd",
        Ty::Header,
        "",
        "The header: the format version (1). A file of a newer version is refused by name.",
    ),
    f(
        "source",
        Ty::File,
        "the file with the asset's own name and `.wav` or `.ogg`",
        "The sound. An `.ogg` whose own parameters are the asset's - or one with none the game reads (no comment, an encoder's tag) under an asset that states none - is published as it is; otherwise with the asset's parameters said again in it, its sound untouched.",
    )
    .titled("Source"),
    f(
        "name",
        Ty::GameName,
        "the path below `gamedata\\sounds` for an asset that lies there, else `<module id>\\<file name>`",
        "The name the game knows the sound by: it is published as `sounds\\<name>.ogg`.",
    )
    .titled("Game Name"),
    f("type", Ty::WordSet(SOUND_TYPES), "\"no_sound\"", "What NPCs make of it, in the game's own words.")
        .titled("Type")
        .on_surface(),
    f(
        "min_distance",
        Ty::Number { min: 0.0, max: 1_000_000.0 },
        "1",
        "Up to this distance the sound plays at its full volume, metres.",
    )
    .titled("Full Volume Within")
    .unit("m")
    .on_surface(),
    f(
        "max_distance",
        Ty::Number { min: 0.1, max: 1_000_000.0 },
        "300",
        "Beyond this distance it is not heard, metres.",
    )
    .titled("Heard Within")
    .unit("m")
    .on_surface(),
    f("volume", Ty::Number { min: 0.0, max: 10.0 }, "1", "Its base volume.").titled("Volume").on_surface(),
    f(
        "ai_distance",
        Ty::Number { min: 0.1, max: 1_000_000.0 },
        "`max_distance`",
        "How far NPCs hear it, metres.",
    )
    .titled("NPCs Hear It Within")
    .unit("m"),
    f(
        "quality",
        Ty::Number { min: -0.1, max: 1.0 },
        "0",
        "The Vorbis quality a `.wav` source is encoded with.",
    )
    .titled("Encoding Quality"),
    f(
        "copyrighted",
        Ty::Bool,
        "false",
        "A recording that may not be streamed: the game does not play it while its streamer mode is on.",
    )
    .titled("Copyrighted")
    .tip("Not played while the game's streamer mode is on.")
    .on_surface(),
    f(
        "streamer",
        Ty::GameName,
        "none: nothing plays",
        "What plays in its place while the streamer mode is on, by its game name: a sound that is not copyrighted itself - a sound asset's `name` without `copyrighted`, or a sound of the game its own list (`configs\\dead_air_x64_sound_rights.ltx`) does not name. Only for a copyrighted sound.",
    )
    .titled("Plays Instead")
    .tip("What plays in its place while the streamer mode is on: a sound without the mark. Nothing: the sound is left out.")
    .on_surface(),
];
