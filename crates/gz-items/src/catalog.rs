//! The item catalog: the game's own description of what an item's sections hold
//! (`configs\item_catalog.ltx` of the linked game, read through its file index; the editor's
//! bundled copy is the fallback). The game's `docs/dead-air/ITEM_DATA.md` is its format:
//!
//! ```ini
//! [catalog]
//! version = 1
//! [scopes]
//! order = item, hud, upgrade, ...       ; which section a key lives in
//! [scope.hud]
//! title = HUD Section
//! help = The section the item's hud key names: ...
//! [families]
//! order = item, weapon, ...
//! [family.weapon]
//! title = Firearm
//! classes = WP_AK74, ...                 ; the `class =` values of the family
//! like = wpn_ak74, wpn_pm               ; what a new item of the family usually starts from
//! help = ...
//! [groups]
//! order = general, economy, ...
//! [group.economy]
//! title = Economy
//! [item.cost]                           ; [<scope>.<name>]
//! families = *                          ; or a list
//! type = float                          ; bool int float string vec2 vec3 vec4 color list section visual
//!                                       ; texture sound particles motion string_id functor token
//! group = economy
//! title = Cost
//! unit = RU
//! required = true                       ; the game stops without it; or the classes that stop
//! default = 0
//! values = a, b                         ; a token's words
//! common = true                         ; on a form's surface, not under More
//! presence = true                       ; the game only asks whether the line is there
//! section = bullet_manager              ; a `global` key's section of the system config
//! pattern = use<N>_functor              ; a series: <N> digits, <x> a word
//! help = What a trader asks for it.
//! ```
//!
//! No key, family or scope is known to the code by name, but for the two the item asset itself
//! writes: [`ITEM`] (its own section, `set`) and [`HUD`] (the section its `hud` key names,
//! `hud.set`). Values are read as written (the lossless reader), so a sentence keeps its blanks.

use gz_ltx::{LineItem, LtxDoc, unquote};
use rustc_hash::FxHashMap;
use serde::Serialize;

/// The catalog's path in the game's files.
pub const PATH: &str = "configs\\item_catalog.ltx";

/// The scope of an item's own section: what the asset's `set` writes.
pub const ITEM: &str = "item";

/// The scope of the section an item's `hud` key names: what the asset's `hud.set` writes.
pub const HUD: &str = "hud";

/// What a key's value is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyType {
    Bool,
    Int,
    Float,
    String,
    Vec2,
    Vec3,
    Vec4,
    Color,
    List,
    Section,
    Visual,
    Texture,
    Sound,
    Particles,
    Motion,
    StringId,
    Functor,
    Token,
    /// A word this editor does not know: any value is taken.
    Other(String),
}

impl KeyType {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bool" => KeyType::Bool,
            "int" => KeyType::Int,
            "float" => KeyType::Float,
            "string" | "" => KeyType::String,
            "vec2" => KeyType::Vec2,
            "vec3" => KeyType::Vec3,
            "vec4" => KeyType::Vec4,
            "color" => KeyType::Color,
            "list" => KeyType::List,
            "section" => KeyType::Section,
            "visual" => KeyType::Visual,
            "texture" => KeyType::Texture,
            "sound" => KeyType::Sound,
            "particles" => KeyType::Particles,
            "motion" => KeyType::Motion,
            "string_id" => KeyType::StringId,
            "functor" => KeyType::Functor,
            "token" => KeyType::Token,
            other => KeyType::Other(other.to_owned()),
        }
    }

    /// The catalog's word for it.
    #[must_use]
    pub fn word(&self) -> &str {
        match self {
            KeyType::Bool => "bool",
            KeyType::Int => "int",
            KeyType::Float => "float",
            KeyType::String => "string",
            KeyType::Vec2 => "vec2",
            KeyType::Vec3 => "vec3",
            KeyType::Vec4 => "vec4",
            KeyType::Color => "color",
            KeyType::List => "list",
            KeyType::Section => "section",
            KeyType::Visual => "visual",
            KeyType::Texture => "texture",
            KeyType::Sound => "sound",
            KeyType::Particles => "particles",
            KeyType::Motion => "motion",
            KeyType::StringId => "string_id",
            KeyType::Functor => "functor",
            KeyType::Token => "token",
            KeyType::Other(w) => w,
        }
    }
}

/// Which section a key lives in: the item's own, its HUD's, one the item names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScopeInfo {
    pub id: String,
    pub title: String,
    pub help: String,
}

/// A family of items, told by the engine class of its section.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Family {
    pub id: String,
    pub title: String,
    /// The `class` values of the family, as written.
    pub classes: Vec<String>,
    /// Sections of the game a new item of the family usually starts from.
    pub like: Vec<String>,
    pub help: String,
}

/// A group of keys, as a form shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Group {
    pub id: String,
    pub title: String,
}

/// A part of a key pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Digits,
    Word,
}

/// A numbered or suffixed series of keys: `use<N>_functor`, `anm_<name>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub text: String,
    parts: Vec<Part>,
}

impl Pattern {
    #[must_use]
    pub fn new(text: &str) -> Self {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find('<') {
            let Some(close) = rest.get(open..).and_then(|r| r.find('>')).map(|c| c + open) else { break };
            if open > 0 {
                parts.push(Part::Text(rest.get(..open).unwrap_or("").to_owned()));
            }
            let name = rest.get(open + 1..close).unwrap_or("");
            parts.push(if name == "N" || name == "n" { Part::Digits } else { Part::Word });
            rest = rest.get(close + 1..).unwrap_or("");
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        Self { text: text.to_owned(), parts }
    }

    /// Whether `key` is of the series.
    #[must_use]
    pub fn matches(&self, key: &str) -> bool {
        fn go(parts: &[Part], s: &str) -> bool {
            let Some((first, rest)) = parts.split_first() else { return s.is_empty() };
            match first {
                Part::Text(t) => s.strip_prefix(t.as_str()).is_some_and(|r| go(rest, r)),
                Part::Digits | Part::Word => {
                    let ok = |c: char| match first {
                        Part::Digits => c.is_ascii_digit(),
                        _ => c.is_ascii_alphanumeric() || c == '_',
                    };
                    let run = s.chars().take_while(|c| ok(*c)).count();
                    (1..=run).rev().any(|n| s.get(n..).is_some_and(|r| go(rest, r)))
                }
            }
        }
        go(&self.parts, key)
    }
}

/// What the catalog says of one key of a scope.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KeyInfo {
    /// The scope's id (`item`, `hud`, ...).
    pub scope: String,
    /// The key, or the series' name when `pattern` is set.
    pub key: String,
    #[serde(skip)]
    pub pattern: Option<Pattern>,
    /// The families it is of; empty: every family.
    pub families: Vec<String>,
    #[serde(rename = "type")]
    pub ty: KeyType,
    pub group: String,
    pub title: String,
    pub unit: String,
    /// The game stops without it (after inheritance), for every class that reads it, or - when
    /// `required_classes` names some - for those classes alone.
    pub required: bool,
    pub required_classes: Vec<String>,
    pub default: Option<String>,
    /// A token's words.
    pub values: Vec<String>,
    /// On a form's surface rather than under More.
    pub common: bool,
    /// The game asks only whether the line is there; its value is not read.
    pub presence: bool,
    /// The system config's section a key of a game-wide scope lives in.
    pub section: Option<String>,
    pub help: String,
}

impl KeyInfo {
    /// Whether the key is of `family` (`None`: an item of no family the catalog knows).
    #[must_use]
    pub fn of_family(&self, family: Option<&str>) -> bool {
        self.families.is_empty()
            || family.is_some_and(|f| self.families.iter().any(|x| x.eq_ignore_ascii_case(f)))
    }

    /// Whether an item of the engine class `class` stops without the key.
    #[must_use]
    pub fn required_for(&self, class: Option<&str>) -> bool {
        self.required
            && (self.required_classes.is_empty()
                || class
                    .is_some_and(|c| self.required_classes.iter().any(|x| x.eq_ignore_ascii_case(c.trim()))))
    }

    /// Whether the key, or its series, is `key`.
    #[must_use]
    pub fn names(&self, key: &str) -> bool {
        match &self.pattern {
            Some(p) => p.matches(key),
            None => self.key == key,
        }
    }
}

/// The whole catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Catalog {
    pub version: u32,
    pub scopes: Vec<ScopeInfo>,
    pub families: Vec<Family>,
    pub groups: Vec<Group>,
    pub keys: Vec<KeyInfo>,
    /// What the reader passed by: a scope nobody declared, a type it does not know.
    pub problems: Vec<String>,
}

fn list(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned).collect()
}

fn yes(v: &str) -> bool {
    gz_ltx::doc::parse_bool(v)
}

impl Catalog {
    /// Reads a catalog's text in one pass. Never fails: what cannot be read is a problem.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let doc = LtxDoc::parse(text);
        // every section's lines, by its name as written; a section written twice reads once
        let mut order: Vec<String> = Vec::new();
        let mut lines: FxHashMap<String, Vec<(String, String)>> = FxHashMap::default();
        let mut current: Option<String> = None;
        for item in doc.items() {
            match item {
                LineItem::Section(name) => {
                    let lc = name.to_ascii_lowercase();
                    if lines.contains_key(&lc) {
                        current = None;
                        continue;
                    }
                    lines.insert(lc.clone(), Vec::new());
                    order.push(name.to_owned());
                    current = Some(lc);
                }
                LineItem::Entry { key, value, .. } => {
                    if let Some(s) = current.as_ref().and_then(|c| lines.get_mut(c)) {
                        s.push((key.to_owned(), unquote(value).trim().to_owned()));
                    }
                }
                LineItem::Other => {}
            }
        }
        let get = |section: &str, key: &str| -> Option<String> {
            lines
                .get(&section.to_ascii_lowercase())
                .and_then(|l| l.iter().rev().find(|(k, _)| k == key))
                .map(|(_, v)| v.clone())
        };
        let ordered = |list_section: &str, prefix: &str| -> Vec<String> {
            let mut ids: Vec<String> = get(list_section, "order").map(|v| list(&v)).unwrap_or_default();
            for h in &order {
                if let Some(id) = h.strip_prefix(prefix)
                    && !ids.iter().any(|x| x.eq_ignore_ascii_case(id))
                {
                    ids.push(id.to_owned());
                }
            }
            ids
        };
        let mut c = Catalog {
            version: get("catalog", "version").and_then(|v| v.parse().ok()).unwrap_or(1),
            ..Catalog::default()
        };
        for id in ordered("scopes", "scope.") {
            let s = format!("scope.{id}");
            c.scopes.push(ScopeInfo {
                title: get(&s, "title").unwrap_or_else(|| id.clone()),
                help: get(&s, "help").unwrap_or_default(),
                id,
            });
        }
        let declared = !c.scopes.is_empty();
        for id in ordered("families", "family.") {
            let s = format!("family.{id}");
            c.families.push(Family {
                title: get(&s, "title").unwrap_or_else(|| id.clone()),
                classes: get(&s, "classes").map(|v| list(&v)).unwrap_or_default(),
                like: get(&s, "like").map(|v| list(&v)).unwrap_or_default(),
                help: get(&s, "help").unwrap_or_default(),
                id,
            });
        }
        for id in ordered("groups", "group.") {
            let s = format!("group.{id}");
            c.groups.push(Group { title: get(&s, "title").unwrap_or_else(|| id.clone()), id });
        }
        for h in &order {
            let Some((scope, key)) = h.split_once('.') else { continue };
            if matches!(scope, "scope" | "family" | "group") || key.is_empty() {
                continue;
            }
            if declared && !c.scopes.iter().any(|s| s.id == scope) {
                c.problems.push(format!("[{h}]: '{scope}' is no scope of [scopes]"));
            } else if !declared && !c.scopes.iter().any(|s| s.id == scope) {
                c.scopes.push(ScopeInfo {
                    id: scope.to_owned(),
                    title: scope.to_owned(),
                    help: String::new(),
                });
            }
            let families = get(h, "families").map(|v| list(&v)).unwrap_or_default();
            let families = if families.iter().any(|f| f == "*") { Vec::new() } else { families };
            let ty = KeyType::parse(&get(h, "type").unwrap_or_default());
            if let KeyType::Other(w) = &ty {
                c.problems
                    .push(format!("[{h}]: the type '{w}' is not one this editor knows; any value is taken"));
            }
            let required = get(h, "required").unwrap_or_default();
            let (required, required_classes) = if required.is_empty() {
                (false, Vec::new())
            } else if yes(&required) {
                (true, Vec::new())
            } else if ["false", "no", "off", "0"].iter().any(|w| required.eq_ignore_ascii_case(w)) {
                (false, Vec::new())
            } else {
                (true, list(&required))
            };
            c.keys.push(KeyInfo {
                scope: scope.to_owned(),
                key: key.to_owned(),
                pattern: get(h, "pattern").filter(|p| !p.is_empty()).map(|p| Pattern::new(&p)),
                families,
                ty,
                group: get(h, "group").unwrap_or_default(),
                title: get(h, "title").unwrap_or_else(|| key.to_owned()),
                unit: get(h, "unit").unwrap_or_default(),
                required,
                required_classes,
                default: get(h, "default"),
                values: get(h, "values").map(|v| list(&v)).unwrap_or_default(),
                common: get(h, "common").is_some_and(|v| yes(&v)),
                presence: get(h, "presence").is_some_and(|v| yes(&v)),
                section: get(h, "section").filter(|s| !s.is_empty()),
                help: get(h, "help").unwrap_or_default(),
            });
        }
        c
    }

    /// The family whose `classes` holds `class` (any case).
    #[must_use]
    pub fn family_of_class(&self, class: &str) -> Option<&Family> {
        let class = class.trim();
        self.families.iter().find(|f| f.classes.iter().any(|c| c.eq_ignore_ascii_case(class)))
    }

    #[must_use]
    pub fn family(&self, id: &str) -> Option<&Family> {
        self.families.iter().find(|f| f.id.eq_ignore_ascii_case(id))
    }

    #[must_use]
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id.eq_ignore_ascii_case(id))
    }

    #[must_use]
    pub fn scope(&self, id: &str) -> Option<&ScopeInfo> {
        self.scopes.iter().find(|s| s.id == id)
    }

    /// What the catalog says of a key of a scope for an item of `family`: the key itself before
    /// a series it belongs to.
    #[must_use]
    pub fn key(&self, scope: &str, family: Option<&str>, key: &str) -> Option<&KeyInfo> {
        let fits = |k: &&KeyInfo| k.scope == scope && k.of_family(family);
        self.keys.iter().filter(fits).find(|k| k.pattern.is_none() && k.key == key).or_else(|| {
            self.keys.iter().filter(fits).find(|k| k.pattern.as_ref().is_some_and(|p| p.matches(key)))
        })
    }

    /// The keys of a scope for an item of `family` (`None`: every one of the scope), in the
    /// catalog's order.
    pub fn keys_of<'a>(
        &'a self,
        scope: &'a str,
        family: Option<&'a str>,
    ) -> impl Iterator<Item = &'a KeyInfo> + 'a {
        self.keys.iter().filter(move |k| k.scope == scope && k.of_family(family))
    }
}

/// Whether `value` (as a config line holds it) is a value of `ty`; `Err` says why not.
pub fn check_value(ty: &KeyType, value: &str, values: &[String]) -> Result<(), String> {
    let v = value.trim();
    let unq = v.trim_matches('"');
    let numbers = |n: usize, what: &str| -> Result<(), String> {
        let parts: Vec<&str> = unq.split(',').map(str::trim).collect();
        if parts.len() != n || parts.iter().any(|p| p.parse::<f64>().is_err()) {
            return Err(format!("takes {what}: {n} numbers separated by commas"));
        }
        Ok(())
    };
    match ty {
        KeyType::Bool => {
            let l = unq.to_ascii_lowercase();
            if ["true", "false", "on", "off", "yes", "no", "1", "0"].contains(&l.as_str()) {
                Ok(())
            } else {
                Err("takes true or false".into())
            }
        }
        KeyType::Int => {
            let digits = unq.strip_prefix(['-', '+']).unwrap_or(unq);
            if !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()) {
                Ok(())
            } else {
                Err("takes a whole number".into())
            }
        }
        KeyType::Float => {
            if unq.parse::<f64>().is_ok_and(f64::is_finite) {
                Ok(())
            } else {
                Err("takes a number".into())
            }
        }
        KeyType::Vec2 => numbers(2, "a vector"),
        KeyType::Vec3 => numbers(3, "a vector"),
        KeyType::Vec4 => numbers(4, "a vector"),
        KeyType::Color => {
            let parts: Vec<&str> = unq.split(',').map(str::trim).collect();
            if (3..=4).contains(&parts.len()) && parts.iter().all(|p| p.parse::<f64>().is_ok()) {
                Ok(())
            } else {
                Err("takes a colour: r, g, b or r, g, b, a".into())
            }
        }
        KeyType::Token if !values.is_empty() => {
            if values.iter().any(|w| w.eq_ignore_ascii_case(unq)) {
                Ok(())
            } else {
                Err(format!("takes one of {}", values.join(", ")))
            }
        }
        KeyType::Functor => {
            if unq.is_empty() || unq.contains('.') {
                Ok(())
            } else {
                Err("takes a script function: module.function".into())
            }
        }
        _ => Ok(()),
    }
}

/// The names a value of `ty` refers to, to be looked up: every member of a list for a
/// section, the first one for a model, a texture, a sound or a particle effect (what follows
/// it is its parameters), every motion of a motion list.
#[must_use]
pub fn named(ty: &KeyType, value: &str) -> Vec<String> {
    let items: Vec<String> =
        value.split(',').map(|x| x.trim().trim_matches('"').to_owned()).filter(|x| !x.is_empty()).collect();
    match ty {
        KeyType::Section | KeyType::Motion => items,
        KeyType::Visual | KeyType::Texture | KeyType::Sound | KeyType::Particles | KeyType::StringId => {
            items.into_iter().take(1).collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "[catalog]\nversion = 1\n[scopes]\norder = item, hud, global\n[scope.hud]\ntitle = HUD Section\nhelp = The section the hud key names.\n[families]\norder = weapon, item\n[family.item]\ntitle = Plain item\nclasses = II_FOOD\nhelp = Junk.\n[family.weapon]\ntitle = Weapon\nclasses = WP_AK74, W_PM\nlike = wpn_ak74\n[groups]\norder = economy\n[group.economy]\ntitle = Economy\n[item.cost]\nfamilies = *\ntype = float\ngroup = economy\ntitle = Cost\nunit = RU\nrequired = true\ncommon = true\nhelp = What a trader asks for it before his own prices.\n[item.snd_shoot_n]\nfamilies = weapon\ntype = sound\npattern = snd_shoot<N>\n[item.af_radius]\ntype = float\nrequired = DET_SIMP, D_SIMDET\npresence = true\n[hud.hands_position]\ntype = vec3\n[global.air_resistance_k]\ntype = float\nsection = bullet_manager\n[odd.x]\ntype = int\n[item.PDM_disp_base]\ntype = float\n";

    #[test]
    fn reads_what_the_file_holds() {
        let c = Catalog::parse(TEXT);
        assert_eq!(c.scopes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["item", "hud", "global"]);
        assert_eq!(c.scope(HUD).unwrap().title, "HUD Section");
        assert_eq!(c.families.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["weapon", "item"]);
        assert_eq!(c.family_of_class("wp_ak74").map(|f| f.id.as_str()), Some("weapon"));
        assert_eq!(c.family("item").unwrap().help, "Junk.");
        let cost = c.key(ITEM, Some("weapon"), "cost").unwrap();
        assert!(cost.required && cost.common && cost.required_for(Some("W_PM")) && cost.required_for(None));
        assert_eq!(cost.help, "What a trader asks for it before his own prices.");
        assert!(c.key(ITEM, Some("weapon"), "snd_shoot12").is_some());
        assert!(c.key(ITEM, Some("weapon"), "snd_shoot").is_none());
        assert!(c.key(ITEM, Some("item"), "snd_shoot1").is_none());
        let af = c.key(ITEM, None, "af_radius").unwrap();
        assert!(af.presence && af.required_for(Some("d_simdet")) && !af.required_for(Some("II_FOOD")));
        assert_eq!(c.key(HUD, None, "hands_position").unwrap().ty, KeyType::Vec3);
        assert_eq!(
            c.key("global", None, "air_resistance_k").unwrap().section.as_deref(),
            Some("bullet_manager")
        );
        // a key keeps its case, as the game reads it
        assert!(c.key(ITEM, None, "PDM_disp_base").is_some());
        assert_eq!(c.problems, ["[odd.x]: 'odd' is no scope of [scopes]"]);
        assert_eq!(c.group("economy").unwrap().title, "Economy");
    }

    #[test]
    fn values_of_a_type() {
        assert!(check_value(&KeyType::Int, "-12", &[]).is_ok());
        assert!(check_value(&KeyType::Int, "1.5", &[]).is_err());
        assert!(check_value(&KeyType::Float, "0.4", &[]).is_ok());
        assert!(check_value(&KeyType::Bool, "On", &[]).is_ok());
        assert!(check_value(&KeyType::Bool, "maybe", &[]).is_err());
        assert!(check_value(&KeyType::Vec3, "0.02, -0.01, 0.1", &[]).is_ok());
        assert!(check_value(&KeyType::Vec3, "0.02, -0.01", &[]).is_err());
        assert!(check_value(&KeyType::Token, "b", &["a".into(), "b".into()]).is_ok());
        assert!(check_value(&KeyType::Token, "c", &["a".into(), "b".into()]).is_err());
        assert!(check_value(&KeyType::Color, "255, 0, 0", &[]).is_ok());
        assert_eq!(named(&KeyType::Sound, "weapons\\ak74_shoot, 1, 0"), ["weapons\\ak74_shoot"]);
        assert_eq!(named(&KeyType::Section, "a, b"), ["a", "b"]);
        let p = Pattern::new("anm_<x>_<N>");
        assert!(p.matches("anm_idle_2") && p.matches("anm_reload_empty_10") && !p.matches("anm_idle"));
        assert!(
            Pattern::new("<snd_key><N>").matches("snd_shoot12")
                && !Pattern::new("<snd_key><N>").matches("snd_shoot")
        );
        assert!(
            Pattern::new("af_sound_<N>_").matches("af_sound_3_")
                && !Pattern::new("af_sound_<N>_").matches("af_sound_3")
        );
        assert!(Pattern::new("<N>icon_layer").matches("1icon_layer"));
    }

    /// The copy the editor ships is the game's catalog, and it reads whole.
    #[test]
    fn the_bundled_catalog_reads_without_a_problem() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gamedata/configs/item_catalog.ltx");
        let text = std::fs::read(&path).unwrap_or_default();
        let c = Catalog::parse(&gz_base::text::decode_guess(&text));
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert!(c.keys.len() > 800, "{} keys", c.keys.len());
        assert!(c.keys.iter().all(|k| !matches!(k.ty, KeyType::Other(_))));
        assert!(c.keys.iter().all(|k| c.scope(&k.scope).is_some()));
        assert!(
            c.keys.iter().all(|k| k.group.is_empty() || c.group(&k.group).is_some()),
            "a key of an undeclared group"
        );
        assert!(
            c.keys.iter().all(|k| k.families.iter().all(|f| c.family(f).is_some())),
            "a key of an undeclared family"
        );
        assert!(c.scope(ITEM).is_some() && c.scope(HUD).is_some());
        // every class is in one family at most
        let mut seen = std::collections::HashSet::new();
        for f in &c.families {
            for class in &f.classes {
                assert!(seen.insert(class.to_ascii_uppercase()), "{class} is in two families");
            }
        }
        assert!(c.key(ITEM, Some("weapon"), "cost").is_some_and(|k| k.required));
    }
}
