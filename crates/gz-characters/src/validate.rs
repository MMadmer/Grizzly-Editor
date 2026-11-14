//! Coded problems of a character, the way quests report them: `E<nnn>` stops a build, `W<nnn>`
//! is a warning (`EditorCharacters::Validate`). A problem is the C++ line: `<code> <where>:
//! <sentence>`, where is the project-relative file.

use std::sync::Arc;

use gz_nq::{PickerType, Pickers};

use crate::game::model_path;
use crate::model::{Character, text_empty, text_in, text_languages, valid_id};

/// The linked game's models, as validation asks about them.
pub trait Models {
    /// The game holds a file at this path (`EditorGameContent::Find`).
    fn exists(&self, rel: &str) -> bool;
    /// The bone names of a skeletal model; empty when it is not skeletal or does not read.
    fn bones(&self, rel: &str) -> Arc<[String]>;
    /// The bones a stalker's damage section names.
    fn damage_bones(&self) -> &[String];
}

/// The lookups of the linked game and the project, as validation asks them (`NqPickers`).
pub trait Lookups {
    /// A linked game backs the lists (`NqPickers::Available`).
    fn available(&self) -> bool;
    /// The list of a type holds anything (`!NqPickers::Index(t).empty()`).
    fn has_any(&self, t: PickerType) -> bool;
    fn known(&self, t: PickerType, id: &str) -> bool;
}

impl Lookups for Pickers {
    fn available(&self) -> bool {
        Pickers::available(self)
    }

    fn has_any(&self, t: PickerType) -> bool {
        !self.entries(t).is_empty()
    }

    fn known(&self, t: PickerType, id: &str) -> bool {
        Pickers::known(self, t, id)
    }
}

/// What validation reads besides the character.
#[derive(Clone, Copy, Default)]
pub struct Cx<'a> {
    /// A game is linked and its files are mounted: the checks that need the game run.
    pub linked: bool,
    pub pickers: Option<&'a dyn Lookups>,
    /// The linked game's trade categories; empty without one.
    pub trade_categories: &'a [String],
    pub models: Option<&'a dyn Models>,
    /// The sections the module itself brings (its `gamedata\configs`): an item named there is
    /// known whatever the linked game says.
    pub own: Option<&'a gz_mod::SectionSet>,
}

/// Whether the module itself brings `section`.
fn own_has(cx: &Cx<'_>, section: &str) -> bool {
    cx.own.is_some_and(|o| o.has(section))
}

/// Whether every character of a text has a cp1251 form; the first six that do not.
fn outside_cp1251(text: &str) -> Option<String> {
    gz_mod::text::outside_cp1251(text)
}

/// A model on a skeleton of its own - Clear Sky's Forester has no eyes and no jaw - passes every
/// other check and stops the game the moment the NPC comes online (`CheckSkeleton`).
fn check_skeleton(models: &dyn Models, rel: &str, place: &str, problems: &mut Vec<String>) {
    let have = models.bones(rel);
    // not skeletal or unreadable: nothing to tell
    if have.is_empty() {
        return;
    }
    let missing: Vec<&str> = models
        .damage_bones()
        .iter()
        .filter(|bone| !have.iter().any(|h| h.eq_ignore_ascii_case(bone)))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        problems.push(format!(
            "E123 {place}: the model has no bone {} - an NPC's damage names them and the game stops the moment it comes online; pick a model on the stalker skeleton",
            missing.join(", ")
        ));
    }
}

/// The problems of one RESOLVED character, appended to `problems`.
pub fn validate(c: &Character, cx: &Cx<'_>, problems: &mut Vec<String>) {
    let place = if c.file.is_empty() { "character" } else { c.file.as_str() };

    if !valid_id(&c.id) {
        problems.push(format!(
            "E102 {place}: id '{}' is not usable (a-z, 0-9 and _, not starting with a digit, up to 48)",
            c.id
        ));
    }

    // a template is not a character: it exists to be inherited from, so it owes the player nothing
    if !c.is_abstract && text_empty(&c.name) {
        problems.push(format!("E103 {place}: no name - it is what the player reads"));
    }

    if !c.is_abstract && c.visual.is_empty() {
        problems.push(format!("E104 {place}: no visual - an NPC without a model cannot spawn"));
    } else if !c.visual.is_empty()
        && cx.linked
        && let Some(models) = cx.models
    {
        // a model the project brings itself is staged by the build and is not expected in the
        // linked game, hence a warning and not an error
        let rel = model_path(&c.visual);
        if models.exists(&rel) {
            check_skeleton(models, &rel, place, problems);
        } else {
            problems.push(format!(
                "W105 {place}: the linked game has no model at '{rel}' - check it is staged in the project"
            ));
        }
    }

    // the engine refuses a character of a community it has never heard of; a module can bring
    // a community of its own, hence a warning
    if !c.community.is_empty()
        && cx.linked
        && let Some(p) = cx.pickers
        && p.has_any(PickerType::Community)
        && !p.known(PickerType::Community, &c.community)
    {
        problems.push(format!("W114 {place}: the linked game has no community '{}' - the engine refuses a character of a community it does not know", c.community));
    }

    // picked from lists, but the text and MCP path can name anything: warnings, since a module
    // may bring its own portrait, dialog or item
    let pickers = cx.pickers.filter(|p| cx.linked && p.available());
    if let Some(p) = pickers {
        if !c.icon.is_empty() && !p.known(PickerType::Icon, &c.icon) {
            problems.push(format!("W115 {place}: no portrait '{}' in the linked game or the project - the player sees an empty frame", c.icon));
        }
        if !c.start_dialog.is_empty() && !p.known(PickerType::Dialog, &c.start_dialog) {
            problems.push(format!(
                "W116 {place}: no dialog '{}' in the linked game or the project",
                c.start_dialog
            ));
        }
        for s in &c.supplies {
            if !s.section.is_empty() && !p.known(PickerType::Item, &s.section) && !own_has(cx, &s.section) {
                problems.push(format!(
                    "W117 {place}: neither the linked game nor the module has an item '{}' - the game stops when the NPC spawns",
                    s.section
                ));
            }
        }
    }

    // a text cp1251 cannot carry loses characters in game: the string table is cp1251
    let mut langs = Vec::new();
    text_languages(&c.name, &mut langs);
    // oracle: a language both texts carry is checked (and reported) twice
    text_languages(&c.bio, &mut langs);
    for lang in &langs {
        let text = text_in(&c.name, lang) + &text_in(&c.bio, lang);
        if let Some(bad) = outside_cp1251(&text) {
            problems.push(format!(
                "W107 {place}: '{bad}' does not exist in cp1251 and will not reach the player"
            ));
        }
    }

    for s in &c.supplies {
        if s.section.is_empty() {
            problems.push(format!("E108 {place}: a supply row without a section"));
        }
        // a zero is how an inherited row is taken away, and it is gone by now; a negative one
        // never meant anything
        if s.count < 1 {
            problems.push(format!("E109 {place}: supply '{}' asks for less than one item", s.section));
        }
    }

    if c.has_money && c.money_max < c.money_min {
        problems.push(format!("E110 {place}: money max is below money min"));
    }

    if c.trade.is_trader() {
        validate_trade(c, cx, pickers, place, problems);
    }
}

// a NaN chance or markup must fail the range checks, as the C++ ones did
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn validate_trade(
    c: &Character,
    cx: &Cx<'_>,
    pickers: Option<&dyn Lookups>,
    place: &str,
    problems: &mut Vec<String>,
) {
    let t = &c.trade;
    for (i, r) in t.stock.iter().enumerate() {
        if r.section.is_empty() {
            problems.push(format!("E118 {place}: a stock row without an item"));
        } else if r.count < 1 {
            problems.push(format!("E118 {place}: stock '{}' asks for less than one item", r.section));
        } else if !(r.chance > 0.0 && r.chance <= 1.0) {
            problems.push(format!("E118 {place}: stock '{}' has a chance outside 0..1", r.section));
        }
        if !r.section.is_empty() && t.stock.iter().take(i).any(|o| o.section.eq_ignore_ascii_case(&r.section))
        {
            problems.push(format!("E118 {place}: '{}' is in the stock twice - one row per item", r.section));
        }
    }
    if !(t.markup > 0.0) {
        problems.push(format!(
            "E119 {place}: the markup must be above zero - it is what the trader asks of an item's cost"
        ));
    }
    if t.pays < 0.0 {
        problems.push(format!("E119 {place}: the trader cannot pay less than nothing"));
    }
    if t.restock < 1 {
        problems.push(format!("E119 {place}: the stock comes back after at least one game hour"));
    }
    if cx.linked {
        let cats = cx.trade_categories;
        let mut check = |list: &[String], what: &str| {
            if cats.is_empty() {
                return;
            }
            for name in list {
                if !cats.iter().any(|k| k == name) {
                    problems.push(format!(
                        "W120 {place}: the linked game has no trade category '{name}' the trader {what} - the trade presets name weapon, drugs, food and the like"
                    ));
                }
            }
        };
        check(&t.sells, "sells");
        check(&t.buys, "buys");
        if let Some(p) = pickers {
            for r in &t.stock {
                if !r.section.is_empty() && !p.known(PickerType::Item, &r.section) && !own_has(cx, &r.section)
                {
                    problems.push(format!(
                        "W121 {place}: neither the linked game nor the module has an item '{}' for the stock",
                        r.section
                    ));
                }
            }
        }
    }
    if t.stock.is_empty() && t.sells.is_empty() {
        problems
            .push(format!("W122 {place}: the trader has nothing to sell - give it a stock or a category"));
    }
}
