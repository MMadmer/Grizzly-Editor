//! The check of one item, positioned at the key it is about. The problems of its own shape are
//! the reader's (`I427`), those that make the game leave it out the composition's (`I401`..`I405`,
//! `I407`..`I410`, `I429`); this is the rest:
//!
//! | Code | Severity | What |
//! |---|---|---|
//! | `I411` | error | A key the game stops without, which the section does not get from its bases. |
//! | `I412` | warning | A value that is no value of the key's type. |
//! | `I413`..`I419` | warning | A model, sound, texture, particle effect, section, string id or motion nobody has. |
//! | `I420` | warning | A text with characters the game's code page lacks. |
//! | `I421` | error | An icon picture that is not there, lies outside the project or is no picture. |
//! | `I422` | warning | An icon rectangle outside its texture. |
//! | `I426` | note | No game is linked: what only the game knows was not looked up. |
//! | `I431` | warning | A use scene's `like` on an item that has a scene already: the game keeps the scene it has. |
//! | `I433` | error | A row of `sections` that makes or changes the section of another item asset. |
//!
//! And of a table file - a trader, a loot table, recipes ([`validate_table`]):
//!
//! | Code | Severity | What |
//! |---|---|---|
//! | `I412` | warning | A chance outside 0 to 1. |
//! | `I417` | warning | An item a row names that is no section of the game or of the module. |
//! | `I423` | warning | A recipe's result, tool or part that is not there. |
//! | `I424` | warning | A trader whose config is not there, or lacks the stock section. |
//! | `I425` | warning | A death loot community that is not there. |
//! | `I430` | warning | A drop of an item the module makes without a count: the death manager never spawns it. |

use std::path::Path;

use gz_luadata::Seg;

use crate::catalog::{self, Catalog, KeyType, check_value, named};
use crate::compose::{Applied, Composition};
use crate::model::{Changes, Icon, Issue, Item, Kind, Scene};
use crate::tables::{Rows, TableFile, trader_file};

pub use crate::compose::{BAD_ID, BAD_SECTION, BOTH, NEITHER};
pub const TWICE: &str = "I406";
pub const REQUIRED: &str = "I411";
pub const BAD_TYPE: &str = "I412";
pub const NO_MODEL: &str = "I413";
pub const NO_SOUND: &str = "I414";
pub const NO_TEXTURE: &str = "I415";
pub const NO_PARTICLES: &str = "I416";
pub const NO_SECTION: &str = "I417";
pub const NO_STRING: &str = "I418";
pub const NO_MOTION: &str = "I419";
pub const CODE_PAGE: &str = "I420";
pub const BAD_PICTURE: &str = "I421";
pub const OUTSIDE_TEXTURE: &str = "I422";
pub const NO_PART: &str = "I423";
pub const NO_CONFIG: &str = "I424";
pub const NO_COMMUNITY: &str = "I425";
pub const UNLINKED: &str = "I426";
pub const NEVER_DROPS: &str = "I430";
pub const LIKE_IGNORED: &str = "I431";
pub const NOT_YOURS: &str = "I433";
pub const NOT_NAMED: &str = "I434";

/// What only the linked game and the project know. Blocking to ask (a model's motions are read
/// on first need): check on the IO pool.
pub trait Lookups: Sync {
    /// A game is linked: the rest is looked up.
    fn linked(&self) -> bool;
    /// `meshes\<name>.ogf`, of the game, the project or a model asset.
    fn model(&self, name: &str) -> bool;
    /// `textures\<name>.dds`, of the game, the project or a texture asset.
    fn texture(&self, name: &str) -> bool;
    /// A texture's size in pixels, when it can be read.
    fn texture_size(&self, name: &str) -> Option<(u32, u32)>;
    /// `sounds\<name>.ogg`, of the game, the project or a sound asset.
    fn sound(&self, name: &str) -> bool;
    /// A particle effect or group of the game or the project.
    fn particle(&self, name: &str) -> bool;
    /// A string id of the game's or the project's string tables.
    fn string_id(&self, id: &str) -> bool;
    /// A community, or a community and a rank (`stalker_veteran`).
    fn community(&self, name: &str) -> bool;
    /// The sections (lower case) of a config under `configs`; `None` when neither the game nor
    /// the project has it.
    fn config_sections(&self, file: &str) -> Option<Vec<String>>;
    /// The motions a model has; `None` when it cannot be read.
    fn motions(&self, visual: &str) -> Option<Vec<String>>;
    /// A picture's size, or why it is none.
    fn picture(&self, path: &Path) -> Result<(u32, u32), String>;
}

/// No game: nothing is looked up, everything passes.
pub struct NoGame;

impl Lookups for NoGame {
    fn linked(&self) -> bool {
        false
    }
    fn model(&self, _: &str) -> bool {
        true
    }
    fn texture(&self, _: &str) -> bool {
        true
    }
    fn texture_size(&self, _: &str) -> Option<(u32, u32)> {
        None
    }
    fn sound(&self, _: &str) -> bool {
        true
    }
    fn particle(&self, _: &str) -> bool {
        true
    }
    fn string_id(&self, _: &str) -> bool {
        true
    }
    fn community(&self, _: &str) -> bool {
        true
    }
    fn config_sections(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn motions(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn picture(&self, path: &Path) -> Result<(u32, u32), String> {
        crate::atlas::picture_size(path)
    }
}

/// What a check reads besides the item.
pub struct Cx<'a> {
    pub catalog: &'a Catalog,
    pub lookups: &'a dyn Lookups,
    /// The project's folder: where pictures are.
    pub root: &'a Path,
    pub module: &'a str,
    /// The other items' sections and the strings they make: what a key may name.
    pub module_strings: &'a [String],
    /// The game's tree without the module: a section of it that works without a key the
    /// catalog calls required shows the game does not stop without it.
    pub game: Option<&'a gz_ltx::Ltx>,
}

fn at(keys: &[&str]) -> Vec<Seg> {
    keys.iter().map(|k| Seg::key(k)).collect()
}

/// The sections the module's `#file` patches of `file` name, but for those of `source` (an
/// item's own rows name what they want, not what is there).
fn patched_sections(comp: &Composition, file: &str, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in comp.file_patches.iter().filter(|p| p.file == file && !p.source.eq_ignore_ascii_case(source)) {
        for d in &p.directives {
            if let gz_ltx::Op::Section { name, .. } = &d.op {
                out.push(name.to_ascii_lowercase());
            }
        }
    }
    out
}

/// The keys of a section this item writes, checked against the catalog: their type and what
/// they name.
#[allow(clippy::too_many_arguments)]
fn check_values(
    out: &mut Vec<Issue>,
    changes: &Changes,
    base_path: &[Seg],
    scope: &str,
    family: Option<&str>,
    comp: &Composition,
    cx: &Cx<'_>,
    motions: &dyn Fn() -> Option<Vec<String>>,
) {
    for (key, v) in &changes.set {
        let Some(info) = cx.catalog.key(scope, family, key) else { continue };
        // the game only asks whether the line is there
        if info.presence {
            continue;
        }
        let mut path = base_path.to_vec();
        path.extend([Seg::key("set"), Seg::key(key)]);
        let text = v.ltx();
        if let Err(why) = check_value(&info.ty, &text, &info.values) {
            out.push(Issue::warning(BAD_TYPE, path.clone(), format!("'{key}' is \"{text}\": it {why}")));
            continue;
        }
        let names = named(&info.ty, &text);
        let linked = cx.lookups.linked();
        for (at, n) in names.into_iter().enumerate() {
            if n.starts_with('$') {
                continue;
            }
            // a HUD motion line is `hands cycle[, item cycle[, speed]]`: the first is a motion of
            // the hands' rig, which many files feed; only the second is the item model's own
            if info.ty == KeyType::Motion && (at != 1 || n.parse::<f32>().is_ok()) {
                continue;
            }
            let (known, code, what) = match &info.ty {
                KeyType::Section => {
                    (comp.ltx.section(&n.to_ascii_lowercase()).is_some(), NO_SECTION, "section")
                }
                KeyType::Visual if linked => (cx.lookups.model(&n), NO_MODEL, "model"),
                // a sound key may name a section of layered sounds instead of a file
                KeyType::Sound if linked => (
                    cx.lookups.sound(&n) || comp.ltx.section(&n.to_ascii_lowercase()).is_some(),
                    NO_SOUND,
                    "sound",
                ),
                KeyType::Texture if linked => (cx.lookups.texture(&n), NO_TEXTURE, "texture"),
                KeyType::Particles if linked => (cx.lookups.particle(&n), NO_PARTICLES, "particle effect"),
                KeyType::StringId if linked => (
                    cx.lookups.string_id(&n) || cx.module_strings.iter().any(|s| s.eq_ignore_ascii_case(&n)),
                    NO_STRING,
                    "string id",
                ),
                KeyType::Motion if linked => match motions() {
                    Some(list) => (list.iter().any(|m| m.eq_ignore_ascii_case(&n)), NO_MOTION, "motion"),
                    None => (true, NO_MOTION, "motion"),
                },
                _ => continue,
            };
            if !known {
                let whose = if *what == *"motion" {
                    " the item's HUD model does not have"
                } else {
                    " neither the game nor the project has"
                };
                out.push(Issue::warning(
                    code,
                    path.clone(),
                    format!("'{key}' names the {what} '{n}', which{whose}"),
                ));
            }
        }
    }
}

/// `I434`: a section the item makes that is not named after the module. A section is one name
/// for the whole game: another module that makes it too is left out, or its item is (the game keeps
/// the first). An item's section left out is `<module>_<id>`, which is.
fn named_after_module(out: &mut Vec<Issue>, item: &Item, module: &str) {
    if module.is_empty() {
        return;
    }
    let say = |name: &str| {
        format!(
            "[{name}] is a new section and not named after the module: a section is one name for the whole game, and of two modules whose items make [{name}] the game keeps the first and leaves the other's item out - name it '{}', or carry the module id as a word of it",
            gz_mod::names::own_form(name, module)
        )
    };
    if item.edit.is_none()
        && let Some(section) = item.section.as_deref().filter(|s| !s.is_empty())
        && !gz_mod::names::is_own(section, module)
    {
        out.push(Issue::warning(NOT_NAMED, at(&["section"]), say(section)));
    }
    for (row, s) in item.sections.iter().enumerate() {
        if !s.name.is_empty() && !gz_mod::names::is_own(&s.name, module) {
            out.push(Issue::warning(
                NOT_NAMED,
                vec![Seg::key("sections"), Seg::Index(row), Seg::key("name")],
                say(&s.name),
            ));
        }
    }
}

fn texts(out: &mut Vec<Issue>, item: &Item) {
    for (key, v) in
        [("name", &item.name), ("short_name", &item.short_name), ("description", &item.description)]
    {
        let mut langs = Vec::new();
        gz_characters::text_languages(v, &mut langs);
        for lang in langs {
            let text = gz_characters::text_raw(v, &lang);
            if !gz_base::text::fits_cp1251(&text) {
                let path = if v.is_string() { at(&[key]) } else { at(&[key, &lang]) };
                out.push(Issue::warning(
                    CODE_PAGE,
                    path,
                    format!("the {lang} text of '{key}' has characters the game's code page (cp1251) lacks: the player reads '?' there"),
                ));
            }
        }
    }
}

/// Whether one of `bases` is a section of the game that lacks `key` too: the game holds it
/// and does not stop, so the catalog's `required` does not reach its class.
fn works_without(cx: &Cx<'_>, bases: &[String], key: &str) -> bool {
    cx.game.is_some_and(|g| {
        bases.iter().any(|b| g.section(&b.to_ascii_lowercase()).is_some_and(|s| s.get(key).is_none()))
    })
}

/// The HUD sections a new item's HUD section starts from: its own `base`, else the HUD its bases
/// name in the game.
fn hud_bases(cx: &Cx<'_>, item: &Item) -> Vec<String> {
    if let Some(h) = item.hud.as_ref().filter(|h| !h.base.is_empty()) {
        return h.base.clone();
    }
    let Some(g) = cx.game else { return Vec::new() };
    item.base
        .iter()
        .filter_map(|b| g.section(&b.to_ascii_lowercase()))
        .filter_map(|s| s.get("hud").and_then(|it| it.value.as_deref()).map(str::to_owned))
        .collect()
}

/// The problems of one item; `applied` and `comp` are what the composition made of it.
#[must_use]
pub fn validate(item: &Item, applied: &Applied, comp: &Composition, cx: &Cx<'_>) -> Vec<Issue> {
    let mut out = Vec::new();
    out.extend(applied.issues.iter().cloned());
    texts(&mut out, item);
    named_after_module(&mut out, item, cx.module);
    // the keys and values go to the game through its code page too
    let mut sets: Vec<(Vec<Seg>, &Changes)> = vec![(Vec::new(), &item.changes)];
    if let Some(h) = &item.hud {
        sets.push((at(&["hud"]), &h.changes));
    }
    for (base, c) in sets {
        for (k, v) in &c.set {
            if !gz_base::text::fits_cp1251(k) || !gz_base::text::fits_cp1251(&v.ltx()) {
                let mut path = base.clone();
                path.extend([Seg::key("set"), Seg::key(k)]);
                out.push(Issue::warning(
                    CODE_PAGE,
                    path,
                    format!(
                        "'{k}' has characters the game's code page (cp1251) lacks: the game reads '?' there"
                    ),
                ));
            }
        }
    }
    if !cx.lookups.linked() {
        out.push(Issue::new(
            UNLINKED,
            gz_base::Severity::Note,
            Vec::new(),
            "no game is linked: the models, sounds, textures, particle effects and string ids the item names were not looked up",
        ));
    }
    if applied.left_out || matches!(item.kind(), Kind::Neither | Kind::Both) {
        return out;
    }

    let section = applied.section.as_str();
    let main = comp.ltx.section(&section.to_ascii_lowercase());
    let class = main.and_then(|s| s.get("class")).and_then(|it| it.value.as_deref()).map(str::trim);
    let family = class.and_then(|c| cx.catalog.family_of_class(c)).map(|f| f.id.clone());
    let family = family.as_deref();

    // what the game stops without: a new item that is no template, after its bases
    if item.kind() == Kind::New && !item.is_abstract() {
        for info in
            cx.catalog.keys_of(catalog::ITEM, family).filter(|k| k.required_for(class) && k.pattern.is_none())
        {
            if main.is_some_and(|s| s.get(&info.key).is_none()) && !works_without(cx, &item.base, &info.key) {
                out.push(Issue::error(
                    REQUIRED,
                    at(&["base"]),
                    format!("the game stops without '{}' ({}), and nothing the item starts from gives it: set it (`set = {{ {} = ... }}`)", info.key, info.title, info.key),
                ));
            }
        }
        if let Some(h) = applied.hud.as_deref() {
            let hs = comp.ltx.section(&h.to_ascii_lowercase());
            for info in cx
                .catalog
                .keys_of(catalog::HUD, family)
                .filter(|k| k.required_for(class) && k.pattern.is_none())
            {
                if hs.is_some_and(|s| s.get(&info.key).is_none())
                    && !works_without(cx, &hud_bases(cx, item), &info.key)
                {
                    out.push(Issue::error(
                        REQUIRED,
                        at(&["hud"]),
                        format!(
                            "the game stops without '{}' ({}) in the HUD section '{h}': set it in `hud.set`",
                            info.key, info.title
                        ),
                    ));
                }
            }
        }
    }

    // the HUD model's motions, read once if a motion is named
    let hud_visual = applied
        .hud
        .as_deref()
        .and_then(|h| comp.ltx.section(&h.to_ascii_lowercase()))
        .and_then(|s| s.get("item_visual"))
        .and_then(|it| it.value.as_deref())
        .map(|v| v.trim().to_owned());
    let cache = std::cell::OnceCell::new();
    let motions = || -> Option<Vec<String>> {
        cache.get_or_init(|| hud_visual.as_deref().and_then(|v| cx.lookups.motions(v))).clone()
    };
    check_values(&mut out, &item.changes, &[], catalog::ITEM, family, comp, cx, &motions);
    if let Some(h) = &item.hud {
        check_values(&mut out, &h.changes, &[Seg::key("hud")], catalog::HUD, family, comp, cx, &motions);
    }

    // the icon
    match &item.icon {
        Some(Icon::Picture { file, .. }) => {
            let asset = cx.root.join(item.file.replace('/', "\\"));
            let path = gz_media::files::beside(&asset, file);
            let place = if matches!(item.icon, Some(Icon::Picture { plain: true, .. })) {
                at(&["icon"])
            } else {
                at(&["icon", "file"])
            };
            if !gz_media::files::inside(cx.root, &path) {
                out.push(Issue::error(
                    BAD_PICTURE,
                    place,
                    format!("the icon picture lies outside the project: {file}"),
                ));
            } else if !path.is_file() {
                out.push(Issue::error(
                    BAD_PICTURE,
                    place,
                    format!("the icon picture is not there: {file} (beside the asset)"),
                ));
            } else if let Err(why) = cx.lookups.picture(&path) {
                out.push(Issue::error(
                    BAD_PICTURE,
                    place,
                    format!("the icon picture cannot be read: {file}: {why}"),
                ));
            }
        }
        Some(Icon::Cell { texture: Some(t), x, y, w, h }) if cx.lookups.linked() => {
            let (w, h) = (w.unwrap_or(1), h.unwrap_or(1));
            if !cx.lookups.texture(t) {
                out.push(Issue::warning(
                    NO_TEXTURE,
                    at(&["icon", "texture"]),
                    format!("the icon's texture '{t}' is neither the game's nor the project's"),
                ));
            } else if let Some((pw, ph)) = cx.lookups.texture_size(t)
                && ((x + w) * crate::atlas::CELL > pw || (y + h) * crate::atlas::CELL > ph)
            {
                out.push(Issue::warning(
                    OUTSIDE_TEXTURE,
                    at(&["icon"]),
                    format!("the rectangle {x}, {y}, {w} x {h} cells reaches past the texture '{t}' ({pw} x {ph} px, {} x {} cells)", pw / crate::atlas::CELL, ph / crate::atlas::CELL),
                ));
            }
        }
        _ => {}
    }

    // a scene starts from the scene of another item: a section of the scenes' config
    if let Some(Scene::Like(like) | Scene::Table { like: Some(like), .. }) = &item.scene
        && cx.lookups.linked()
        && let Some(mut have) = cx.lookups.config_sections(crate::compose::SCENES)
    {
        have.extend(patched_sections(comp, crate::compose::SCENES, &item.file));
        let path =
            if matches!(item.scene, Some(Scene::Like(_))) { at(&["scene"]) } else { at(&["scene", "like"]) };
        // the game makes a section once: `like` is what a NEW scene starts from
        if have.iter().any(|h| h.eq_ignore_ascii_case(&applied.section)) {
            out.push(Issue::warning(
                LIKE_IGNORED,
                path.clone(),
                format!(
                    "[{}] has a use scene already: 'like' is what a new scene starts from, and this one keeps its own",
                    applied.section
                ),
            ));
        }
        if !have.iter().any(|h| h.eq_ignore_ascii_case(like)) {
            out.push(Issue::warning(
                NO_SECTION,
                path,
                format!(
                    "'{like}' has no use scene in configs\\{}: there is nothing to start from",
                    crate::compose::SCENES
                ),
            ));
        }
    }
    // a scene's own keys go to the game through its code page too
    if let Some(Scene::Table { set, .. }) = &item.scene {
        for (k, v) in set {
            if !gz_base::text::fits_cp1251(k) || !gz_base::text::fits_cp1251(&v.ltx()) {
                out.push(Issue::warning(
                    CODE_PAGE,
                    vec![Seg::key("scene"), Seg::key("set"), Seg::key(k)],
                    format!(
                        "'{k}' has characters the game's code page (cp1251) lacks: the game reads '?' there"
                    ),
                ));
            }
        }
    }
    out
}

/// What the check of a table file reads besides the file.
pub struct TableCx<'a> {
    pub lookups: &'a dyn Lookups,
    /// The game's tree without the module: an item of it is the game's.
    pub game: Option<&'a gz_ltx::Ltx>,
    /// The items the module's loot tables give a count (lower case), by any of their rows.
    pub counted: &'a rustc_hash::FxHashSet<String>,
}

/// The problems of one table file; `comp` is the module composed with it. Its own shape is the
/// reader's (`I427`).
#[must_use]
pub fn validate_table(table: &TableFile, comp: &Composition, cx: &TableCx<'_>) -> Vec<Issue> {
    let mut out = Vec::new();
    let has = |s: &str| comp.ltx.section(&s.trim().to_ascii_lowercase()).is_some();
    let linked = cx.lookups.linked();
    if !linked {
        out.push(Issue::new(
            UNLINKED,
            gz_base::Severity::Note,
            Vec::new(),
            "no game is linked: the traders' configs and the communities the file names were not looked up",
        ));
    }
    let chance = |out: &mut Vec<Issue>, path: Vec<Seg>, c: f64| {
        if !(0.0..=1.0).contains(&c) {
            out.push(Issue::warning(BAD_TYPE, path, format!("a chance of {c}: a chance is from 0 to 1")));
        }
    };
    let unknown_item = |out: &mut Vec<Issue>, path: Vec<Seg>, item: &str| {
        if !item.trim().is_empty() && !has(item) {
            out.push(Issue::warning(
                NO_SECTION,
                path,
                format!("'{item}' is no section of the game or of the module: there is no such item"),
            ));
        }
    };
    match &table.rows {
        Rows::Trade { trader, stock } => {
            let file = trader_file(trader);
            // the stock sections the trader's config has, with those the module's other files add
            let have = match (&file, linked) {
                (Some(f), true) => match cx.lookups.config_sections(f) {
                    None => {
                        out.push(Issue::warning(
                            NO_CONFIG,
                            at(&["trader"]),
                            format!("the trader '{trader}' is neither the game's nor the project's: there is no configs\\{f}"),
                        ));
                        None
                    }
                    Some(mut have) => {
                        have.extend(patched_sections(comp, f, &table.file));
                        Some(have)
                    }
                },
                _ => None,
            };
            for (i, s) in stock.iter().enumerate() {
                let row = vec![Seg::key("stock"), Seg::Index(i)];
                let key = |k: &str| {
                    let mut p = row.clone();
                    p.push(Seg::key(k));
                    p
                };
                unknown_item(&mut out, key("item"), &s.item);
                if let (Some(have), Some(f)) = (&have, &file) {
                    for sup in s.sections().iter().filter(|x| !have.iter().any(|h| h.eq_ignore_ascii_case(x)))
                    {
                        let place = if s.supplies.is_empty() { row.clone() } else { key("supplies") };
                        out.push(Issue::warning(
                            NO_CONFIG,
                            place,
                            format!("the trader's config configs\\{f} has no stock section [{sup}]"),
                        ));
                    }
                }
                if let Some(c) = s.chance {
                    chance(&mut out, key("chance"), c);
                }
            }
        }
        Rows::Loot(drops) => {
            for (i, l) in drops.iter().enumerate() {
                let row = vec![Seg::key("drops"), Seg::Index(i)];
                let key = |k: &str| {
                    let mut p = row.clone();
                    p.push(Seg::key(k));
                    p
                };
                unknown_item(&mut out, key("item"), &l.item);
                let item = l.item.trim().to_ascii_lowercase();
                // the game's own items have their counts; one the module makes needs a row that gives it
                let games = cx.game.is_some_and(|g| g.section(&item).is_some());
                if has(&item) && !games && !cx.counted.contains(&item) {
                    out.push(Issue::warning(
                        NEVER_DROPS,
                        row.clone(),
                        format!("no drop of '{item}' has a count: the death manager never spawns an item without one"),
                    ));
                }
                if let Some(c) = l.chance {
                    chance(&mut out, key("chance"), c);
                }
                for (who, c) in &l.by {
                    let mut p = key("by");
                    p.push(Seg::key(who));
                    if linked && !cx.lookups.community(who) {
                        out.push(Issue::warning(
                            NO_COMMUNITY,
                            p.clone(),
                            format!(
                                "'{who}' is no community of the game, nor a community and one of its ranks"
                            ),
                        ));
                    }
                    chance(&mut out, p, *c);
                }
            }
        }
        Rows::Recipes(list) => {
            for (i, c) in list.iter().enumerate() {
                let row = [Seg::key("recipes"), Seg::Index(i)];
                let mut miss = |key: &str, what: &str, name: &str| {
                    if !name.trim().is_empty() && !has(name) {
                        let mut p = row.to_vec();
                        p.push(Seg::key(key));
                        out.push(Issue::warning(
                            NO_PART,
                            p,
                            format!(
                                "the recipe's {what} '{name}' is no section of the game or of the module"
                            ),
                        ));
                    }
                };
                miss("makes", "result", &c.makes);
                miss("at", "tool", &c.at);
                for part in &c.parts {
                    miss("parts", "part", part);
                }
            }
        }
    }
    out
}
