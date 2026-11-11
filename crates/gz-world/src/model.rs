//! `[world]` and `[start_kit]` of `mod.ltx`: read from the manifest document's raw lines the way
//! the C++ `EditorMod::Load` read them, written back as the C++ `Save` wrote them.

use gz_mod::doc::entry_line;
use gz_mod::text::split_list;
use gz_mod::{Edit, ManifestDoc};
use serde_json::{Map, Value, json};

/// The five rules, in the order the file and the answers spell them (`WorldRuleKey`).
pub const RULES: [&str; 5] = ["original_npcs", "random_stalkers", "mutants", "npc_quests", "story_quests"];

/// The rows of the new-game screen a mode can offer.
pub const MAX_PLAYER_CHOICES: usize = 3;

/// The pad of a key in `[world]` and `[start_kit]` (`kWorldKeyWidth`).
pub const KEY_WIDTH: usize = 15;

pub const WORLD: &str = "world";
pub const START_KIT: &str = "start_kit";

/// The rule of a key, any case (`WorldRuleFromKey`).
#[must_use]
pub fn rule_index(key: &str) -> Option<usize> {
    RULES.iter().position(|r| r.eq_ignore_ascii_case(key))
}

/// The engine's reading: only an explicit no changes anything.
fn is_no(val: &str) -> bool {
    matches!(val, "false" | "0" | "no" | "off")
}

/// C's `atoi`.
#[must_use]
pub fn atoi(s: &str) -> i32 {
    let v = gz_nq::text::c_atoi(s);
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// `key = value` of a raw line, both trimmed; a line without `=` is all key.
fn split_line(line: &str) -> (&str, &str) {
    fn t(s: &str) -> &str {
        s.trim_matches([' ', '\t', '\r', '\n'])
    }
    match line.find('=') {
        Some(eq) => (t(line.get(..eq).unwrap_or("")), t(line.get(eq + 1..).unwrap_or(""))),
        None => (t(line), ""),
    }
}

/// What of the original game the module's mode keeps (`SWorld`). The engine reads it; it
/// takes effect on a new game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    /// Per rule: the game's content of that kind stays.
    pub keep: [bool; 5],
    /// Per rule: the new-game screen lets the player change it.
    pub player_choice: [bool; 5],
    /// Where the rules apply; empty = everywhere.
    pub levels: Vec<String>,
    /// The only levels the player may reach; empty = anywhere.
    pub area: Vec<String>,
    /// Every transition on these levels starts closed.
    pub locked_levels: Vec<String>,
    /// These transitions (level changer names) start closed.
    pub locked: Vec<String>,
    /// Unknown `[world]` lines, kept as written.
    pub extra: Vec<String>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            keep: [true; 5],
            player_choice: [false; 5],
            levels: Vec::new(),
            area: Vec::new(),
            locked_levels: Vec::new(),
            locked: Vec::new(),
            extra: Vec::new(),
        }
    }
}

impl World {
    /// `[world]` of a manifest; the stock world without one.
    #[must_use]
    pub fn parse(doc: &ManifestDoc) -> Self {
        Self::from_lines(doc.section_lines(WORLD))
    }

    /// The world of raw `[world]` lines, comments already cut.
    #[must_use]
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut w = Self::default();
        for line in lines {
            let (key, val) = split_line(line);
            if let Some(rule) = rule_index(key) {
                if let Some(k) = w.keep.get_mut(rule) {
                    *k = !is_no(val);
                }
            } else if key == "levels" {
                w.levels.extend(split_list(val, ','));
            } else if key == "area" {
                w.area.extend(split_list(val, ','));
            } else if key == "locked_levels" {
                w.locked_levels.extend(split_list(val, ','));
            } else if key == "locked" {
                w.locked.extend(split_list(val, ','));
            } else if key == "player_choice" {
                for name in split_list(val, ',') {
                    if let Some(c) = rule_index(&name).and_then(|r| w.player_choice.get_mut(r)) {
                        *c = true;
                    }
                }
            } else {
                w.extra.push(line.to_owned());
            }
        }
        w
    }

    /// The world is the stock game's: the section is then not written (`SWorld::Empty`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keep.iter().all(|k| *k)
            && !self.player_choice.iter().any(|c| *c)
            && self.levels.is_empty()
            && self.area.is_empty()
            && self.locked_levels.is_empty()
            && self.locked.is_empty()
            && self.extra.is_empty()
    }

    #[must_use]
    pub fn choice_count(&self) -> usize {
        self.player_choice.iter().filter(|c| **c).count()
    }

    /// The rule keys the player may change, in rule order.
    #[must_use]
    pub fn choices(&self) -> Vec<&'static str> {
        RULES.iter().zip(self.player_choice).filter(|(_, on)| *on).map(|(k, _)| *k).collect()
    }

    /// The section's lines as `Save` writes them: every rule spelled out once the section exists,
    /// so the file reads as the whole decision, not as a diff against defaults nobody sees.
    /// Empty for the stock world.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = RULES
            .iter()
            .zip(self.keep)
            .map(|(k, keep)| entry_line(k, if keep { "true" } else { "false" }, KEY_WIDTH))
            .collect();
        let joined = |key: &str, v: &[String], out: &mut Vec<String>| {
            if !v.is_empty() {
                out.push(entry_line(key, &v.join(", "), KEY_WIDTH));
            }
        };
        let choices: Vec<String> = self.choices().into_iter().map(str::to_owned).collect();
        joined("player_choice", &choices, &mut out);
        joined("levels", &self.levels, &mut out);
        joined("area", &self.area, &mut out);
        joined("locked_levels", &self.locked_levels, &mut out);
        joined("locked", &self.locked, &mut out);
        out.extend(self.extra.iter().cloned());
        out
    }

    /// The edit that makes the manifest say this world; the stock world takes the section out.
    #[must_use]
    pub fn edit(&self) -> Edit {
        Edit::lines(WORLD, self.lines())
    }

    /// What closes a transition at the start of a game, in the engine's order
    /// (`XmsWorld::TransitionOpen`): the changer itself, its level, the area (`ClosedBy`).
    #[must_use]
    pub fn closed_by(&self, name: &str, level: &str, to_level: &str) -> &'static str {
        if self.locked.iter().any(|l| l == name) {
            "locked"
        } else if self.locked_levels.iter().any(|l| l == level) {
            "level"
        } else if !self.area.is_empty() && !self.area.iter().any(|l| l == to_level) {
            "area"
        } else {
            ""
        }
    }

    /// `{"rules":{...},"player_choice":[...],"levels":[...],"area":[...],"locked_levels":[...],
    /// "locked":[...],"declared":...}` (`JsonWorld`).
    #[must_use]
    pub fn json(&self) -> Value {
        let mut rules = Map::new();
        for (k, keep) in RULES.iter().zip(self.keep) {
            rules.insert((*k).to_owned(), Value::Bool(keep));
        }
        json!({
            "rules": rules,
            "player_choice": self.choices(),
            "levels": self.levels,
            "area": self.area,
            "locked_levels": self.locked_levels,
            "locked": self.locked,
            "declared": !self.is_empty(),
        })
    }
}

/// Adds or takes out a name of a list (`Toggle`).
pub fn toggle(v: &mut Vec<String>, name: &str, on: bool) {
    let at = v.iter().position(|x| x == name);
    match (on, at) {
        (true, None) => v.push(name.to_owned()),
        (false, Some(i)) => {
            v.remove(i);
        }
        _ => {}
    }
}

/// One row of a start kit: an item section and how many of it (ammo: boxes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitItem {
    pub section: String,
    pub count: i32,
}

/// What a new game of the module's mode begins with (`SStartKit`). It REPLACES the new-game
/// screen's loadout. The section being there is what declares a kit: an empty one starts the
/// player with nothing but the PDA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartKit {
    pub declared: bool,
    /// -1 = not written: the new-game screen's money stays.
    pub money: i32,
    /// The perks picked on the new-game screen stay.
    pub perks: bool,
    /// In the order written.
    pub items: Vec<KitItem>,
}

impl Default for StartKit {
    fn default() -> Self {
        Self { declared: false, money: -1, perks: true, items: Vec::new() }
    }
}

impl StartKit {
    /// `[start_kit]` of a manifest: DA's own loadout format - a bare section is one of it,
    /// `section = N` is N, `money` and `perks` are the two words that are not items.
    #[must_use]
    pub fn parse(doc: &ManifestDoc) -> Self {
        let mut k = Self::from_lines(doc.section_lines(START_KIT));
        k.declared = doc.has_section(START_KIT);
        k
    }

    /// The kit of raw lines (declared).
    #[must_use]
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut k = Self { declared: true, ..Self::default() };
        for line in lines {
            let (key, val) = split_line(line);
            if key == "money" {
                k.money = atoi(val).max(0);
            } else if key == "perks" {
                k.perks = !is_no(val);
            } else if !key.is_empty() {
                k.items.push(KitItem {
                    section: key.to_owned(),
                    count: if val.is_empty() { 1 } else { atoi(val) },
                });
            }
        }
        k
    }

    /// The section's lines as `Save` writes them: money and perks, then the items in the
    /// author's order.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.money >= 0 {
            out.push(entry_line("money", &self.money.to_string(), KEY_WIDTH));
        }
        if !self.perks {
            out.push(entry_line("perks", "false", KEY_WIDTH));
        }
        for it in self.items.iter().filter(|i| !i.section.is_empty()) {
            out.push(entry_line(&it.section, &it.count.to_string(), KEY_WIDTH));
        }
        out
    }

    /// The edit that makes `doc` say this kit; `None` when it says it already in effect.
    #[must_use]
    pub fn edit(&self, doc: &ManifestDoc) -> Option<Edit> {
        if !self.declared {
            return doc.has_section(START_KIT).then(|| Edit::RemoveSection { section: START_KIT.to_owned() });
        }
        let mut lines = self.lines();
        if lines.is_empty() {
            // an empty section is a declared kit, but the manifest store drops a section that
            // holds nothing: an explicit default keeps it (the game reads the same kit)
            if doc.has_section(START_KIT) && doc.section_lines(START_KIT).is_empty() {
                return None;
            }
            lines.push(entry_line("perks", "true", KEY_WIDTH));
        }
        Some(Edit::lines(START_KIT, lines))
    }

    /// `{"declared":..,"money":N|null,"perks":..,"items":[{"section","count"}]}`; money is null
    /// while the kit leaves it to the new-game screen (`JsonStartKit`).
    #[must_use]
    pub fn json(&self) -> Value {
        let items: Vec<Value> =
            self.items.iter().map(|i| json!({ "section": i.section, "count": i.count })).collect();
        json!({
            "declared": self.declared,
            "money": if self.money >= 0 { json!(self.money) } else { Value::Null },
            "perks": self.perks,
            "items": items,
        })
    }
}

/// The game's own base kit - `[shared]` of `plugins\faction_loadouts.ltx`, what every faction
/// gets before the extra kits: the rows and the money (a money list `1000,3000` gives its first
/// amount). `None` when the text holds neither (`GameBaseKit`).
#[must_use]
pub fn base_kit(text: &str) -> Option<StartKit> {
    let mut out = StartKit { declared: true, ..StartKit::default() };
    let mut shared = false;
    for raw in text.split('\n') {
        let mut line = raw.trim_matches([' ', '\t', '\r', '\n']);
        if let Some(sc) = line.find(';') {
            line = line.get(..sc).unwrap_or("").trim_matches([' ', '\t', '\r', '\n']);
        }
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.find(']').map_or(rest, |c| rest.get(..c).unwrap_or(""));
            shared = name.eq_ignore_ascii_case("shared");
            continue;
        }
        if !shared {
            continue;
        }
        let (key, val) = split_line(line);
        if key == "money" {
            out.money = atoi(val).max(0);
            continue;
        }
        out.items.push(KitItem {
            section: key.to_owned(),
            count: if val.is_empty() { 1 } else { atoi(val).max(1) },
        });
    }
    (!out.items.is_empty() || out.money >= 0).then_some(out)
}
