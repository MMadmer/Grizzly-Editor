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
//! | `I423` | warning | A recipe's tool or part that is not there. |
//! | `I424` | warning | A config a row names that is not there, or lacks the section. |
//! | `I425` | warning | A death loot community that is not there. |
//! | `I426` | note | No game is linked: what only the game knows was not looked up. |
//! | `I430` | warning | A new item's death loot without a count: the death manager never spawns it. |
//! | `I431` | warning | A use scene's `like` on an item that has a scene already: the game keeps the scene it has. |

use std::path::Path;

use gz_luadata::Seg;

use crate::catalog::{self, Catalog, KeyType, check_value, named};
use crate::compose::{Applied, Composition};
use crate::model::{Changes, Icon, Issue, Item, Kind, Scene};

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
            "no game is linked: the models, sounds, textures, particle effects, string ids, communities and traders' files the item names were not looked up",
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

    // where it is found
    for (i, c) in item.craft.iter().enumerate() {
        let row = [Seg::key("craft"), Seg::Index(i)];
        let has = |s: &str| comp.ltx.section(&s.trim().to_ascii_lowercase()).is_some();
        if !c.at.trim().is_empty() && !has(&c.at) {
            let mut p = row.to_vec();
            p.push(Seg::key("at"));
            out.push(Issue::warning(
                NO_PART,
                p,
                format!("the recipe's tool '{}' is no section of the game or of the module", c.at),
            ));
        }
        for part in c.parts.iter().filter(|p| !has(p)) {
            let mut p = row.to_vec();
            p.push(Seg::key("parts"));
            out.push(Issue::warning(
                NO_PART,
                p,
                format!("the recipe's part '{part}' is no section of the game or of the module"),
            ));
        }
    }
    if cx.lookups.linked() {
        for (i, s) in item.sold.iter().enumerate() {
            let row = vec![Seg::key("sold"), Seg::Index(i)];
            let Some(file) = s.file() else { continue };
            match cx.lookups.config_sections(&file) {
                None => out.push(Issue::warning(
                    NO_CONFIG,
                    row,
                    format!(
                        "the trader's config '{}' is neither the game's nor the project's (configs\\{file})",
                        s.by
                    ),
                )),
                Some(mut have) => {
                    have.extend(patched_sections(comp, &file, &item.file));
                    for sup in s.sections().iter().filter(|x| !have.iter().any(|h| h.eq_ignore_ascii_case(x)))
                    {
                        out.push(Issue::warning(
                            NO_CONFIG,
                            row.clone(),
                            format!("the trader's config configs\\{file} has no section [{sup}]"),
                        ));
                    }
                }
            }
        }
        for (i, l) in item.lists.iter().enumerate() {
            let Some(f) = l.file.as_deref().filter(|f| !f.trim().is_empty()) else { continue };
            let file = gz_ltx::config_key(f);
            if cx.lookups.config_sections(&file).is_none() {
                out.push(Issue::warning(
                    NO_CONFIG,
                    vec![Seg::key("lists"), Seg::Index(i), Seg::key("file")],
                    format!("the config configs\\{file} is neither the game's nor the project's"),
                ));
            }
        }
        if let Some(l) = &item.loot {
            for (who, _) in l.by.iter().filter(|(w, _)| !cx.lookups.community(w)) {
                out.push(Issue::warning(
                    NO_COMMUNITY,
                    vec![Seg::key("loot"), Seg::key("by"), Seg::key(who)],
                    format!("'{who}' is no community of the game, nor a community and one of its ranks"),
                ));
            }
        }
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
    if let Some(l) = &item.loot {
        if l.count.is_none() && item.kind() == Kind::New {
            out.push(Issue::warning(
                NEVER_DROPS,
                at(&["loot"]),
                "the death loot has no count (loot.count): the death manager never spawns an item without one",
            ));
        }
        let chances = l
            .chance
            .map(|c| (at(&["loot", "chance"]), c))
            .into_iter()
            .chain(l.by.iter().map(|(w, c)| (vec![Seg::key("loot"), Seg::key("by"), Seg::key(w)], *c)));
        for (path, c) in chances.filter(|(_, c)| !(0.0..=1.0).contains(c)) {
            out.push(Issue::warning(BAD_TYPE, path, format!("a chance of {c}: a chance is from 0 to 1")));
        }
    }
    for (i, s) in item.sold.iter().enumerate() {
        if let Some(c) = s.chance.filter(|c| !(0.0..=1.0).contains(c)) {
            out.push(Issue::warning(
                BAD_TYPE,
                vec![Seg::key("sold"), Seg::Index(i), Seg::key("chance")],
                format!("a chance of {c}: a chance is from 0 to 1"),
            ));
        }
    }
    out
}
