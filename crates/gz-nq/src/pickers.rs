//! The lookups behind the parameter pickers and `W060` (`NqPickers`): items, squads, levels,
//! smarts, story ids, profiles, communities, info portions, map spots, restrictors, spawn
//! sections, dialogs, portraits, ranks, level changers.
//!
//! Built from the linked game - its `system.ltx` tree, its string table, the XML and ltx files
//! the tree does not include, its `all.spawn` - never from a hardcoded list. Captions are what
//! the player reads: items through `inv_name`, NPCs through their profile's `<name>`, levels and
//! communities through their own ids, smarts through `misc\smart_names.ltx`. Story ids and
//! restrictors also come from the project (its `spawn\custom_data` and its levels' `spawn.part`),
//! and the open scene adds its own on top ([`Pickers::compose`]).
//!
//! What is built: [`build_game`] (one linked game; the builders run in parallel on the caller's
//! pool), [`build_project`] (the project's own files), [`Pickers::compose`] (both, the scene on
//! top, in the C++ order).

use gz_fmt_xml::strings::StringTable;
use gz_ltx::{Ltx, Section};
use gz_vfs::GameFiles;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::text::{self, clip_utf8, contains_no_case, fold, split, trim};

/// A picker type, in the C++ order (`EType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PickerType {
    Item,
    Squad,
    Level,
    Smart,
    Story,
    Profile,
    Community,
    Info,
    Spot,
    Restrictor,
    SpawnSection,
    Dialog,
    Icon,
    Rank,
    LevelChanger,
}

const COUNT: usize = 15;

impl PickerType {
    pub const ALL: [PickerType; COUNT] = [
        Self::Item,
        Self::Squad,
        Self::Level,
        Self::Smart,
        Self::Story,
        Self::Profile,
        Self::Community,
        Self::Info,
        Self::Spot,
        Self::Restrictor,
        Self::SpawnSection,
        Self::Dialog,
        Self::Icon,
        Self::Rank,
        Self::LevelChanger,
    ];

    /// The catalog type name (`kTypeNames`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Item => "item_section",
            Self::Squad => "squad_section",
            Self::Level => "level",
            Self::Smart => "smart",
            Self::Story => "story_id",
            Self::Profile => "profile",
            Self::Community => "community",
            Self::Info => "info",
            Self::Spot => "spot_type",
            Self::Restrictor => "restrictor",
            Self::SpawnSection => "spawn_section",
            Self::Dialog => "dialog_id",
            Self::Icon => "character_icon",
            Self::Rank => "rank",
            Self::LevelChanger => "level_changer",
        }
    }

    /// The picker of a catalog type, `None` when the type has none (`TypeFromName`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// One entry of a picker: the id, its caption (may be empty) and an origin hint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickerEntry {
    pub id: String,
    /// UTF-8 caption, empty when the game gives the id no text.
    pub name: String,
    /// Where it comes from: a section, a file, a level, `project`, `spawn ...`.
    pub extra: String,
}

/// A linked game's transition (from its `all.spawn`): the object name the engine knows it by,
/// its level and where it leads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transition {
    pub name: String,
    pub level: String,
    pub to_level: String,
}

type Raw = (PickerType, String, String, String);

#[derive(Clone, Debug, Default)]
struct Lists {
    lists: Vec<Vec<PickerEntry>>,
    known: Vec<FxHashSet<String>>,
}

impl Lists {
    fn new() -> Self {
        Self { lists: vec![Vec::new(); COUNT], known: vec![FxHashSet::default(); COUNT] }
    }

    /// `Add`: the first of an id wins; a caption that repeats the id is dropped.
    fn add(&mut self, t: PickerType, id: &str, name: &str, extra: &str) {
        if id.is_empty() {
            return;
        }
        let (Some(known), Some(list)) = (self.known.get_mut(t.slot()), self.lists.get_mut(t.slot())) else {
            return;
        };
        if !known.insert(id.to_owned()) {
            return;
        }
        list.push(PickerEntry {
            id: id.to_owned(),
            name: if name == id { String::new() } else { name.to_owned() },
            extra: extra.to_owned(),
        });
    }

    fn add_all(&mut self, raw: &[Raw]) {
        for (t, id, name, extra) in raw {
            self.add(*t, id, name, extra);
        }
    }

    fn sort(&mut self, t: PickerType) {
        if let Some(l) = self.lists.get_mut(t.slot()) {
            l.sort_by(|a, b| a.id.cmp(&b.id));
        }
    }

    fn reset_known(&mut self, t: PickerType) {
        if let (Some(k), Some(l)) = (self.known.get_mut(t.slot()), self.lists.get(t.slot())) {
            *k = l.iter().map(|e| e.id.clone()).collect();
        }
    }
}

/// The linked game's part of the pickers, sorted by id.
#[derive(Clone, Debug, Default)]
pub struct GameIndex {
    lists: Lists,
}

impl GameIndex {
    #[must_use]
    pub fn entries(&self, t: PickerType) -> &[PickerEntry] {
        self.lists.lists.get(t.slot()).map_or(&[], Vec::as_slice)
    }
}

/// What the project itself brings: story ids, restrictors, dialogs, portraits, and where its
/// smart terrains stand.
#[derive(Clone, Debug, Default)]
pub struct ProjectPart {
    stories: Vec<Raw>,
    restrictors: Vec<Raw>,
    looks: Vec<Raw>,
    /// `(smart, "level (x, y, z)")`: a place hint for a smart the game lists without a caption.
    smart_hints: Vec<(String, String)>,
}

/// The pickers as one view: the game's index, the project's part, the open scene on top.
#[derive(Clone, Debug, Default)]
pub struct Pickers {
    available: bool,
    lists: Lists,
}

impl Pickers {
    /// Composes the view in the C++ order: the game's index (or the fixed spot list without a
    /// game), then the project's stories, restrictors and looks, the project-fed lists sorted,
    /// then the scene's restrictors and story ids.
    #[must_use]
    pub fn compose(
        game: Option<&GameIndex>,
        project: &ProjectPart,
        scene_restrictors: &[PickerEntry],
        scene_stories: &[PickerEntry],
    ) -> Self {
        let mut lists = match game {
            Some(g) => g.lists.clone(),
            None => {
                let mut l = Lists::new();
                l.add_all(&spots());
                l
            }
        };
        lists.add_all(&project.stories);
        lists.add_all(&project.restrictors);
        lists.add_all(&project.looks);
        if let Some(smarts) = lists.lists.get_mut(PickerType::Smart.slot()) {
            for (name, hint) in &project.smart_hints {
                if let Some(e) = smarts.iter_mut().find(|e| &e.id == name)
                    && e.extra.is_empty()
                {
                    e.extra = hint.clone();
                }
            }
        }
        for t in [PickerType::Story, PickerType::Restrictor, PickerType::Dialog, PickerType::Icon] {
            lists.sort(t);
        }
        for e in scene_restrictors {
            lists.add(PickerType::Restrictor, &e.id, &e.name, &e.extra);
        }
        for e in scene_stories {
            lists.add(PickerType::Story, &e.id, &e.name, &e.extra);
        }
        lists.sort(PickerType::Restrictor);
        lists.reset_known(PickerType::Restrictor);
        Self { available: game.is_some(), lists }
    }

    /// Whether a linked game backs the lists; without one `W060` is not raised.
    #[must_use]
    pub fn available(&self) -> bool {
        self.available
    }

    #[must_use]
    pub fn entries(&self, t: PickerType) -> &[PickerEntry] {
        self.lists.lists.get(t.slot()).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn known(&self, t: PickerType, id: &str) -> bool {
        self.lists.known.get(t.slot()).is_some_and(|k| k.contains(id))
    }

    /// Up to `limit` entries matching `query`: id or caption prefixes first, then substrings;
    /// story ids of placed objects before the respawn templates (`Search`).
    #[must_use]
    pub fn search(&self, t: PickerType, query: &str, limit: usize) -> Vec<&PickerEntry> {
        let limit = if limit == 0 { 100 } else { limit };
        let idx = self.entries(t);
        let fq = fold(query);
        let mut out: Vec<&PickerEntry> = Vec::new();
        let is_prefix = |e: &PickerEntry| {
            if query.is_empty() {
                return true;
            }
            let id = e.id.as_bytes();
            if id.len() >= query.len()
                && id.get(..query.len()).is_some_and(|h| h.eq_ignore_ascii_case(query.as_bytes()))
            {
                return true;
            }
            !e.name.is_empty() && fold(&e.name).starts_with(&fq)
        };
        for pass in 0..2 {
            for e in idx {
                if out.len() >= limit {
                    break;
                }
                let prefix = is_prefix(e);
                if pass == 0 && !prefix {
                    continue;
                }
                if pass == 1
                    && (prefix || !(contains_no_case(&e.id, query) || contains_no_case(&e.name, query)))
                {
                    continue;
                }
                out.push(e);
            }
        }
        if t == PickerType::Story {
            let (mut first, rest): (Vec<&PickerEntry>, Vec<&PickerEntry>) = out
                .into_iter()
                .partition(|e| e.extra.as_bytes().get(..5).is_some_and(|h| h.eq_ignore_ascii_case(b"spawn")));
            first.extend(rest);
            out = first;
        }
        out
    }
}

// ---- the game ---------------------------------------------------------------------------------

/// What [`build_game`] reads.
pub struct GameInput<'a> {
    pub files: &'a GameFiles,
    /// The game's `system.ltx` tree, when it loads.
    pub ltx: Option<&'a Ltx>,
    pub strings: &'a StringTable,
    pub transitions: &'a [Transition],
}

fn read_text(files: &GameFiles, rel: &str) -> String {
    files.read(rel).map(|b| gz_base::text::cp1251_to_utf8(&b).into_owned()).unwrap_or_default()
}

fn read_xml(files: &GameFiles, rel: &str) -> String {
    files.read(rel).map(|b| gz_fmt_xml::decode(&b).into_owned()).unwrap_or_default()
}

fn caption(st: &StringTable, id: &str) -> String {
    st.get(id).map(|t| trim(t).to_owned()).unwrap_or_default()
}

/// A line of a section: present means `Some`, an empty value `Some("")`.
fn line<'s>(s: &'s Section, key: &str) -> Option<&'s str> {
    s.get(key).map(|it| it.value.as_deref().unwrap_or(""))
}

fn ltx_line<'s>(ltx: &'s Ltx, section: &str, key: &str) -> Option<&'s str> {
    ltx.section(section).and_then(|s| line(s, key))
}

fn find_from(text: &str, pat: &str, from: usize) -> Option<usize> {
    text.get(from..).and_then(|r| r.find(pat)).map(|i| from + i)
}

/// The ids of `<tag id="...">` openings with where each tag starts (`ScanIds`).
fn scan_ids(text: &str, tag: &str) -> Vec<(String, usize)> {
    let pat = format!("<{tag}");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(at) = find_from(text, &pat, pos) {
        pos = at + pat.len();
        let after = text.as_bytes().get(pos).copied().unwrap_or(0);
        if !matches!(after, b' ' | b'\t' | b'\n' | b'\r') {
            continue;
        }
        let gt = find_from(text, ">", at);
        let Some(idat) = find_from(text, "id=\"", at) else { continue };
        if gt.is_some_and(|g| idat > g) {
            continue;
        }
        let Some(ide) = find_from(text, "\"", idat + 4) else { break };
        out.push((text.get(idat + 4..ide).unwrap_or("").to_owned(), at));
    }
    out
}

/// Each line of an ltx text, its comment cut, trimmed, empty lines skipped.
fn ltx_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|l| {
            let l = trim(l);
            let l = l.find(';').map_or(l, |c| l.get(..c).unwrap_or(""));
            trim(l)
        })
        .filter(|l| !l.is_empty())
}

fn section_name(line: &str) -> &str {
    let body = line.get(1..).unwrap_or("");
    body.find(']').map_or(body, |c| body.get(..c).unwrap_or(""))
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    Some((trim(line.get(..eq).unwrap_or("")), trim(line.get(eq + 1..).unwrap_or(""))))
}

/// Profiles, the captions of the named ones and every portrait a game character wears
/// (`BuildProfiles`).
fn build_profiles(inp: &GameInput<'_>) -> (Vec<Raw>, FxHashMap<String, String>, FxHashSet<String>) {
    let mut raw = Vec::new();
    let mut captions: FxHashMap<String, String> = FxHashMap::default();
    let mut icons: FxHashSet<String> = FxHashSet::default();
    let Some(ltx) = inp.ltx else { return (raw, captions, icons) };
    let Some(list) = ltx_line(ltx, "profiles", "specific_characters_files") else {
        return (raw, captions, icons);
    };
    for f in split(list, ',') {
        let text = read_xml(inp.files, &format!("configs/gameplay/{f}.xml"));
        if text.is_empty() {
            continue;
        }
        let ids = scan_ids(&text, "specific_character");
        let mut at = find_from(&text, "<icon>", 0);
        while let Some(a) = at {
            let Some(end) = find_from(&text, "</icon>", a) else { break };
            let icon = trim(text.get(a + 6..end).unwrap_or("")).to_ascii_lowercase();
            if !icon.is_empty() {
                icons.insert(icon);
            }
            at = find_from(&text, "<icon>", a + 6);
        }
        for (i, (id, tag_at)) in ids.iter().enumerate() {
            let mut name = String::new();
            let next = ids.get(i + 1).map_or(text.len(), |n| n.1);
            if let Some(nm) = find_from(&text, "<name>", *tag_at).filter(|&nm| nm < next)
                && let Some(ne) = find_from(&text, "</name>", nm)
            {
                let key = trim(text.get(nm + 6..ne).unwrap_or(""));
                name = caption(inp.strings, key);
                if !name.is_empty() {
                    captions.entry(id.clone()).or_insert_with(|| name.clone());
                } else {
                    name = key.to_owned();
                }
            }
            raw.push((PickerType::Profile, id.clone(), name, f.clone()));
        }
    }
    (raw, captions, icons)
}

/// The caption of a spawn section: an NPC by its profile, an item by its `inv_name`
/// (`SectionCaption`).
fn section_caption(
    ltx: Option<&Ltx>,
    sect: &str,
    st: &StringTable,
    profiles: &FxHashMap<String, String>,
) -> String {
    let Some(s) = ltx.and_then(|l| l.section(sect)) else { return String::new() };
    if let Some(prof) = line(s, "character_profile")
        && !prof.is_empty()
        && let Some(c) = profiles.get(prof)
    {
        return c.clone();
    }
    if let Some(key) = line(s, "inv_name")
        && !key.is_empty()
    {
        return caption(st, key);
    }
    String::new()
}

/// Items, spawn sections, squads and the story ids of config sections (`BuildFromSettings`).
fn build_from_settings(inp: &GameInput<'_>, profiles: &FxHashMap<String, String>) -> Vec<Raw> {
    let mut raw = Vec::new();
    let Some(ltx) = inp.ltx else { return raw };
    for s in ltx.sections() {
        let name = s.name.to_string();
        let mut inv_caption = String::new();
        if let Some(v) = line(s, "inv_name") {
            inv_caption = caption(inp.strings, v);
            let shown = if inv_caption.is_empty() { v.to_owned() } else { inv_caption.clone() };
            raw.push((PickerType::Item, name.clone(), shown, String::new()));
        }
        // anything the world can hold: the same $spawn line the Spawn tool lists by
        if let Some(v) = line(s, "$spawn").filter(|v| !v.is_empty()) {
            raw.push((PickerType::SpawnSection, name.clone(), String::new(), v.to_owned()));
        }
        if line(s, "class").is_some_and(|v| v.eq_ignore_ascii_case("ON_OFF_S")) {
            // how many bodies a squad gives has to be answerable without opening the configs
            let mut extra = line(s, "faction").filter(|f| !f.is_empty()).unwrap_or("").to_owned();
            let mut members =
                line(s, "npc").filter(|n| !n.is_empty()).map_or(0, |n| split(n, ',').len() as i64);
            if members == 0
                && let Some(c) = line(s, "npc_count").filter(|c| !c.is_empty())
            {
                members = text::c_atoi(c);
            }
            if members > 0 {
                if !extra.is_empty() {
                    extra.push_str(", ");
                }
                extra.push_str(&format!("{members} NPC"));
            }
            raw.push((PickerType::Squad, name.clone(), String::new(), extra));
        }
        if let Some(sid) = line(s, "story_id").filter(|v| !v.is_empty()) {
            let extra = format!("section {name}");
            let mut cap = String::new();
            if let Some(prof) = line(s, "character_profile").filter(|p| !p.is_empty())
                && let Some(c) = profiles.get(prof)
            {
                cap = c.clone();
            }
            if cap.is_empty() {
                cap = inv_caption.clone();
            }
            // a squad whose roster is one named NPC reads as that NPC
            if cap.is_empty()
                && let Some(npc) = line(s, "npc").filter(|n| !n.is_empty() && !n.contains(','))
            {
                let npc = trim(npc);
                if let Some(np) = ltx_line(ltx, npc, "character_profile").filter(|p| !p.is_empty())
                    && let Some(c) = profiles.get(np)
                {
                    cap = c.clone();
                }
            }
            raw.push((PickerType::Story, sid.to_owned(), cap, extra));
        }
    }
    raw
}

/// The levels of `game_levels.ltx` with their captions (`BuildLevels`).
fn build_levels(inp: &GameInput<'_>) -> Vec<Raw> {
    let mut raw = Vec::new();
    let text = read_text(inp.files, "configs/game_levels.ltx");
    if text.is_empty() {
        return raw;
    }
    let st = inp.strings;
    let resolve = |lvl: &str, key: &str| -> String {
        let mut out = caption(st, lvl);
        if out.is_empty() && !key.is_empty() {
            out = caption(st, key);
        }
        if out.is_empty() {
            out = key.to_owned();
        }
        out
    };
    let (mut sect, mut name, mut cap) = (String::new(), String::new(), String::new());
    for l in ltx_lines(&text) {
        if l.starts_with('[') {
            if !name.is_empty() && sect != "levels" {
                raw.push((PickerType::Level, name.clone(), resolve(&name, &cap), sect.clone()));
            }
            sect = section_name(l).to_owned();
            name.clear();
            cap.clear();
            continue;
        }
        let Some((k, v)) = key_value(l) else { continue };
        let v = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v.get(1..v.len() - 1).unwrap_or("")
        } else {
            v
        };
        if k == "name" {
            name = v.to_owned();
        } else if k == "caption" {
            cap = v.to_owned();
        }
    }
    if !name.is_empty() && sect != "levels" {
        raw.push((PickerType::Level, name.clone(), resolve(&name, &cap), sect));
    }
    raw
}

/// The transitions of the linked game, captioned with where they lead (`BuildLevelChangers`).
fn build_level_changers(transitions: &[Transition], levels: &[Raw]) -> Vec<Raw> {
    transitions
        .iter()
        .map(|t| {
            let to = levels
                .iter()
                .rfind(|(_, id, name, _)| id == &t.to_level && !name.is_empty())
                .map_or_else(|| t.to_level.clone(), |(_, _, name, _)| name.clone());
            (
                PickerType::LevelChanger,
                t.name.clone(),
                format!("to {to}"),
                format!("{} -> {}", t.level, t.to_level),
            )
        })
        .collect()
}

/// The smart terrains of `misc\simulation.ltx`, captioned through `misc\smart_names.ltx`
/// (`BuildSmarts`).
fn build_smarts(inp: &GameInput<'_>) -> Vec<Raw> {
    let mut keys: FxHashMap<String, String> = FxHashMap::default();
    let names = read_text(inp.files, "configs/misc/smart_names.ltx");
    let mut in_levels = false;
    for l in ltx_lines(&names) {
        if l.starts_with('[') {
            in_levels = l.as_bytes().get(..8).is_some_and(|h| h.eq_ignore_ascii_case(b"[levels]"));
            continue;
        }
        if in_levels {
            continue;
        }
        let Some((smart, v)) = key_value(l) else { continue };
        if !smart.is_empty() {
            keys.insert(smart.to_owned(), v.to_owned());
        }
    }
    let sim = read_text(inp.files, "configs/misc/simulation.ltx");
    let mut raw = Vec::new();
    for l in sim.split('\n').map(trim) {
        if !l.starts_with('[') {
            continue;
        }
        let Some(close) = l.find(']') else { continue };
        let id = l.get(1..close).unwrap_or("");
        let cap = keys.get(id).map(|k| caption(inp.strings, k)).unwrap_or_default();
        raw.push((PickerType::Smart, id.to_owned(), cap, String::new()));
    }
    raw
}

/// The communities of `creatures\game_relations.ltx` (`BuildCommunities`).
fn build_communities(inp: &GameInput<'_>, text: &str) -> Vec<Raw> {
    let mut raw = Vec::new();
    let mut sect = String::new();
    for l in ltx_lines(text) {
        if l.starts_with('[') {
            sect = section_name(l).to_owned();
            continue;
        }
        if sect != "communities" && sect != "monster_communities" && sect != "game_relations" {
            continue;
        }
        let Some((k, v)) = key_value(l) else { continue };
        if k != "communities" {
            continue;
        }
        let parts = split(v, ',');
        // name, index, name, index, ...
        let mut i = 0;
        while i + 1 < parts.len() {
            let Some(id) = parts.get(i) else { break };
            // the faction picker labels a community in the plural
            let mut name = caption(inp.strings, &format!("st_faction_{id}"));
            if name.is_empty() {
                name = caption(inp.strings, id);
            }
            raw.push((PickerType::Community, id.clone(), name, sect.clone()));
            i += 2;
        }
    }
    raw
}

/// The ranks of `[game_relations] rating` / `rating_names` (`BuildRanks`): the value where a
/// rank starts is its extra.
fn build_ranks(inp: &GameInput<'_>, text: &str) -> Vec<Raw> {
    let (mut rating, mut names, mut sect) = (String::new(), String::new(), String::new());
    for l in ltx_lines(text) {
        if l.starts_with('[') {
            sect = section_name(l).to_owned();
            continue;
        }
        if sect != "game_relations" {
            continue;
        }
        let Some(eq) = l.find('=') else { continue };
        let key = trim(l.get(..eq).unwrap_or(""));
        if key == "rating" {
            rating = l.get(eq + 1..).unwrap_or("").to_owned();
        } else if key == "rating_names" {
            names = l.get(eq + 1..).unwrap_or("").to_owned();
        }
    }
    let r = split(&rating, ',');
    let n = split(&names, ',');
    let mut raw = Vec::new();
    let mut i = 0;
    while let Some(id) = r.get(i) {
        let from = if i == 0 { 0 } else { r.get(i - 1).map_or(0, |v| text::c_atoi(v)) };
        let cap = n.get(i).map(|k| caption(inp.strings, k)).unwrap_or_default();
        raw.push((PickerType::Rank, id.clone(), cap, from.to_string()));
        i += 2;
    }
    raw
}

fn files_list(ltx: Option<&Ltx>, section: &str, key: &str) -> Vec<String> {
    ltx.and_then(|l| ltx_line(l, section, key)).map(|v| split(v, ',')).unwrap_or_default()
}

/// The info portions of `[info_portions] files` (`BuildInfos`).
fn build_infos(inp: &GameInput<'_>) -> Vec<Raw> {
    let mut raw = Vec::new();
    for f in files_list(inp.ltx, "info_portions", "files") {
        let text = read_xml(inp.files, &format!("configs/gameplay/{f}.xml"));
        for (id, _) in scan_ids(&text, "info_portion") {
            raw.push((PickerType::Info, id, String::new(), f.clone()));
        }
    }
    raw
}

/// The dialogs of one file, captioned with the first line they say (`AddDialogs`).
fn add_dialogs(text: &str, st: Option<&StringTable>, extra: &str, raw: &mut Vec<Raw>) {
    let ids = scan_ids(text, "dialog");
    for (i, (id, at)) in ids.iter().enumerate() {
        let end = ids.get(i + 1).map_or(text.len(), |n| n.1);
        let mut cap = String::new();
        if let Some(t) = find_from(text, "<text>", *at).filter(|&t| t < end)
            && let Some(te) = find_from(text, "</text>", t).filter(|&te| te < end)
        {
            let key = trim(text.get(t + 6..te).unwrap_or(""));
            cap = st.map(|s| caption(s, key)).unwrap_or_default();
            if cap.is_empty() {
                cap = key.to_owned();
            }
            cap = clip_utf8(&cap, 64);
        }
        raw.push((PickerType::Dialog, id.clone(), cap, extra.to_owned()));
    }
}

/// The dialogs of `[dialogs] files` (`BuildDialogs`).
fn build_dialogs(inp: &GameInput<'_>) -> Vec<Raw> {
    let mut raw = Vec::new();
    for f in files_list(inp.ltx, "dialogs", "files") {
        let text = read_xml(inp.files, &format!("configs/gameplay/{f}.xml"));
        if !text.is_empty() {
            add_dialogs(&text, Some(inp.strings), &f, &mut raw);
        }
    }
    raw
}

/// The value of `name="..."` inside one tag.
fn tag_attr(tag: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(at) = find_from(tag, &key, from) {
        from = at + 1;
        // a whole attribute name, not the tail of a longer one
        if at > 0 && !matches!(tag.as_bytes().get(at - 1), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            continue;
        }
        let b = at + key.len();
        return find_from(tag, "\"", b).map(|e| tag.get(b..e).unwrap_or("").to_owned()).unwrap_or_default();
    }
    String::new()
}

struct SheetTexture {
    id: String,
    sheet: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Every `<texture>` of a description with the sheet it is cut from, comments dropped
/// (`ReadTextureDescr`).
fn read_texture_descr(raw: &str) -> Vec<SheetTexture> {
    let mut text = String::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        match find_from(raw, "<!--", pos) {
            None => {
                text.push_str(raw.get(pos..).unwrap_or(""));
                break;
            }
            Some(c) => {
                text.push_str(raw.get(pos..c).unwrap_or(""));
                pos = find_from(raw, "-->", c + 4).map_or(raw.len(), |e| e + 3);
            }
        }
    }
    let mut out = Vec::new();
    let mut sheet = String::new();
    let mut pos = 0;
    loop {
        let f = find_from(&text, "<file", pos);
        let t = find_from(&text, "<texture", pos);
        let at = match (f, t) {
            (None, None) => break,
            (Some(f), None) => f,
            (None, Some(t)) => t,
            (Some(f), Some(t)) => f.min(t),
        };
        let Some(gt) = find_from(&text, ">", at) else { break };
        let tag = text.get(at..gt).unwrap_or("");
        pos = gt + 1;
        if Some(at) == f {
            sheet = tag_attr(tag, "name");
            continue;
        }
        let e = SheetTexture {
            id: tag_attr(tag, "id"),
            sheet: sheet.clone(),
            x: text::c_atoi(&tag_attr(tag, "x")),
            y: text::c_atoi(&tag_attr(tag, "y")),
            w: text::c_atoi(&tag_attr(tag, "width")),
            h: text::c_atoi(&tag_attr(tag, "height")),
        };
        if !e.id.is_empty() && !e.sheet.is_empty() && e.w > 0 && e.h > 0 {
            out.push(e);
        }
    }
    out
}

fn icon_extra(t: &SheetTexture, project: bool) -> String {
    format!("{}{}|{}|{}|{}|{}", if project { "project:" } else { "" }, t.sheet, t.x, t.y, t.w, t.h)
}

/// Portraits: every picture of a portrait sheet, and of any other sheet only those a game
/// character wears (`BuildIcons`).
fn build_icons(inp: &GameInput<'_>, used: &FxHashSet<String>) -> Vec<Raw> {
    const PREFIX: &str = "configs/ui/textures_descr/";
    let mut raw = Vec::new();
    for f in inp.files.find("configs/ui/textures_descr", "xml") {
        let leaf = f.path.get(PREFIX.len()..).unwrap_or("");
        let sheet = leaf.as_bytes().get(..17).is_some_and(|h| h.eq_ignore_ascii_case(b"ui_actor_portrets"))
            || leaf.as_bytes().get(..7).is_some_and(|h| h.eq_ignore_ascii_case(b"ui_npc_"));
        let text = read_xml(inp.files, f.path);
        for t in read_texture_descr(&text) {
            if sheet || used.contains(&t.id.to_ascii_lowercase()) {
                raw.push((PickerType::Icon, t.id.clone(), String::new(), icon_extra(&t, false)));
            }
        }
    }
    raw
}

fn spots() -> Vec<Raw> {
    [
        "secondary_task_location",
        "storyline_task_location",
        "ui_secondary_task_blink",
        "ui_storyline_task_blink",
        "treasure",
        "primary_task_location",
    ]
    .into_iter()
    .map(|s| (PickerType::Spot, s.to_owned(), String::new(), "core".to_owned()))
    .collect()
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The story ids of the objects the game placed, read out of `all.spawn` - a `story_id` line of
/// a config section belongs to a respawn template (`BuildSpawnStories`).
fn build_spawn_stories(inp: &GameInput<'_>, profiles: &FxHashMap<String, String>) -> Vec<Raw> {
    const MARK: &[u8] = b"[story_object]";
    let mut raw = Vec::new();
    let Ok(bytes) = inp.files.read("spawns/all.spawn") else { return raw };
    let end = bytes.len();
    let mut p = 0;
    while p + MARK.len() < end {
        let Some(off) = bytes.get(p..end - MARK.len()).and_then(|r| r.iter().position(|&c| c == b'[')) else {
            break;
        };
        let at = p + off;
        if bytes.get(at..at + MARK.len()) != Some(MARK) {
            p = at + 1;
            continue;
        }
        // the value is on one of the next few lines: "story_id = x", or the bare id
        let mut q = at + MARK.len();
        let stop = (q + 256).min(end);
        while q < stop {
            let mut eol = q;
            while eol < stop && !matches!(bytes.get(eol), Some(b'\n' | b'\r' | 0)) {
                eol += 1;
            }
            let line_text = gz_base::text::cp1251_to_utf8(bytes.get(q..eol).unwrap_or(&[])).into_owned();
            let l = trim(&line_text);
            q = if eol < stop { eol + 1 } else { stop };
            if l.is_empty() {
                continue;
            }
            if l.starts_with('[') {
                break;
            }
            let sid = match key_value(l) {
                Some((k, v)) => {
                    if k != "story_id" {
                        break;
                    }
                    v.to_owned()
                }
                None => l.to_owned(),
            };
            if !sid.is_empty() {
                // the packet before the block and the logic line after it name the object's
                // section; the nearest section with a caption wins
                let back = at.saturating_sub(512);
                let fwd = (at + 512).min(end);
                let mut tokens: Vec<(usize, String)> = Vec::new();
                let mut tok_at = back;
                let mut tok_len = 0usize;
                for b in back..=fwd {
                    let c = if b < fwd { bytes.get(b).copied().unwrap_or(b' ') } else { b' ' };
                    if is_word(c) {
                        if tok_len == 0 {
                            tok_at = b;
                        }
                        tok_len += 1;
                        continue;
                    }
                    if tok_len >= 4 {
                        let d = tok_at.abs_diff(at);
                        let tok = String::from_utf8_lossy(bytes.get(tok_at..tok_at + tok_len).unwrap_or(&[]))
                            .into_owned();
                        tokens.push((d, tok));
                    }
                    tok_len = 0;
                }
                tokens.sort();
                let mut name = String::new();
                let mut from = String::new();
                for (_, tok) in &tokens {
                    name = section_caption(inp.ltx, tok, inp.strings, profiles);
                    if !name.is_empty() {
                        from = tok.clone();
                        break;
                    }
                }
                let mut extra = String::from("spawn");
                if !from.is_empty() {
                    extra.push(' ');
                    extra.push_str(&from);
                }
                raw.push((PickerType::Story, sid, name, extra));
            }
            break;
        }
        p = at + MARK.len();
    }
    raw
}

/// The linked game's part of the pickers (`Build` with a game): the builders run in parallel on
/// the caller's pool and are folded in the C++ order, each list then sorted by id.
#[must_use]
pub fn build_game(inp: &GameInput<'_>) -> GameIndex {
    let (profiles, captions, used_icons) = build_profiles(inp);
    let relations = read_text(inp.files, "configs/creatures/game_relations.ltx");
    let ((stories, settings), ((levels, smarts), ((communities, infos), ((dialogs, icons), ranks)))) =
        rayon::join(
            || rayon::join(|| build_spawn_stories(inp, &captions), || build_from_settings(inp, &captions)),
            || {
                rayon::join(
                    || rayon::join(|| build_levels(inp), || build_smarts(inp)),
                    || {
                        rayon::join(
                            || rayon::join(|| build_communities(inp, &relations), || build_infos(inp)),
                            || {
                                rayon::join(
                                    || rayon::join(|| build_dialogs(inp), || build_icons(inp, &used_icons)),
                                    || build_ranks(inp, &relations),
                                )
                            },
                        )
                    },
                )
            },
        );
    let changers = build_level_changers(inp.transitions, &levels);
    let mut lists = Lists::new();
    // placed objects first: a template id looks the same in the list but resolves to nothing
    for part in [
        &profiles,
        &stories,
        &settings,
        &levels,
        &changers,
        &smarts,
        &communities,
        &infos,
        &dialogs,
        &icons,
        &ranks,
        &spots(),
    ] {
        lists.add_all(part);
    }
    for t in PickerType::ALL {
        lists.sort(t);
    }
    GameIndex { lists }
}

// ---- the project ------------------------------------------------------------------------------

/// The project files [`build_project`] reads, already decoded: `(project-relative path, text)`.
#[derive(Clone, Debug, Default)]
pub struct ProjectFiles {
    /// `spawn\custom_data\**\*.ltx`.
    pub custom_data: Vec<(String, String)>,
    /// `rawdata\levels\**\spawn.part`.
    pub spawn_parts: Vec<(String, String)>,
    /// `gamedata\configs\gameplay\**\*.xml`.
    pub gameplay: Vec<(String, String)>,
    /// `gamedata\configs\ui\textures_descr\**\*.xml`.
    pub textures_descr: Vec<(String, String)>,
}

/// `[story_object] story_id = ...` of the project's custom data (`BuildProjectStories`).
fn project_stories(files: &ProjectFiles, out: &mut Vec<Raw>) {
    for (_, text) in &files.custom_data {
        let mut in_story = false;
        for l in ltx_lines(text) {
            if l.starts_with('[') {
                in_story = l.as_bytes().get(..14).is_some_and(|h| h.eq_ignore_ascii_case(b"[story_object]"));
                continue;
            }
            if !in_story {
                continue;
            }
            let Some((k, v)) = key_value(l) else { continue };
            if k == "story_id" {
                out.push((PickerType::Story, v.to_owned(), String::new(), "project".to_owned()));
            }
        }
    }
}

fn parse_three(s: &str) -> Option<(f32, f32, f32)> {
    let mut it = s.split(',').map(|p| p.trim().parse::<f32>());
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(x)), Some(Ok(y)), Some(Ok(z))) => Some((x, y, z)),
        _ => None,
    }
}

/// One object of a level's spawn: a restrictor is an entry of its own, a smart only a place hint
/// (`NoteSpawnObject`).
fn note_spawn_object(section: &str, name: &str, level: &str, at: &str, part: &mut ProjectPart) {
    if name.is_empty() {
        return;
    }
    let mut hint = level.to_owned();
    if !at.is_empty()
        && let Some((x, y, z)) = parse_three(at)
    {
        hint.push_str(&format!(" ({x:.0}, {y:.0}, {z:.0})"));
    }
    if section == "space_restrictor" {
        part.restrictors.push((PickerType::Restrictor, name.to_owned(), String::new(), hint));
        return;
    }
    if section == "smart_terrain" {
        part.smart_hints.push((name.to_owned(), hint));
    }
}

const ZERO_POS: &str = "0.000000, 0.000000, 0.000000";

/// The restrictors and authored story ids of the project's levels (`BuildProjectRestrictors`):
/// a `spawn.part` is plain ltx text, the object name the runtime looks a zone up by is in it.
fn project_restrictors(files: &ProjectFiles, part: &mut ProjectPart) {
    for (rel, text) in &files.spawn_parts {
        // the level a spawn.part belongs to is its own folder
        let mut segs = rel.rsplit(['/', '\\']);
        segs.next();
        let level = segs.next().unwrap_or("").to_owned();
        let (mut section, mut name, mut at, mut hdr_name, mut hdr_at) =
            (String::new(), String::new(), String::new(), String::new(), String::new());
        let (mut in_spawndata, mut in_header) = (false, false);
        // 0 outside 000024, 1 inside it, 2 inside its [story_object]
        let mut cd = 0;
        for raw_line in text.split('\n') {
            let l = trim(raw_line);
            if l.is_empty() {
                continue;
            }
            if cd != 0 {
                let real_header = l.as_bytes().get(..8).is_some_and(|h| h.eq_ignore_ascii_case(b"[object_"));
                let body = l.trim_end_matches(['"', '\r']);
                let ceq = body.find('=');
                let ckey = ceq.map_or("", |e| trim(body.get(..e).unwrap_or("")));
                let packet_key = ckey.len() == 6 && ckey.bytes().all(|c| c.is_ascii_digit());
                if real_header || packet_key {
                    cd = 0;
                } else {
                    if body.contains("[story_object]") {
                        cd = 2;
                        continue;
                    }
                    if body.starts_with('[') {
                        cd = 1;
                        continue;
                    }
                    if cd == 2 && !body.is_empty() {
                        let mut sid = body;
                        if let Some(e) = ceq {
                            if ckey != "story_id" {
                                cd = 1;
                                continue;
                            }
                            sid = body.get(e + 1..).unwrap_or("");
                        }
                        let sid = trim(sid);
                        if !sid.is_empty() {
                            let who = if name.is_empty() { &hdr_name } else { &name };
                            part.stories.push((
                                PickerType::Story,
                                sid.to_owned(),
                                String::new(),
                                format!("project: {who}"),
                            ));
                        }
                        cd = 1;
                    }
                    continue;
                }
            }
            if l.starts_with('[') {
                let who = if name.is_empty() { hdr_name.clone() } else { name.clone() };
                let pos = if at.is_empty() || at == ZERO_POS { hdr_at.clone() } else { at.clone() };
                note_spawn_object(&section, &who, &level, &pos, part);
                section.clear();
                name.clear();
                at.clear();
                let lower = l.to_ascii_lowercase();
                in_spawndata =
                    l.len() > 11 && lower.starts_with("[object_") && lower.ends_with("_spawndata]");
                in_header = false;
                if !in_spawndata && lower.starts_with("[object_") && l.ends_with(']') {
                    in_header = l.get(8..l.len() - 1).is_some_and(|d| d.bytes().all(|c| c.is_ascii_digit()));
                    if in_header {
                        hdr_name.clear();
                        hdr_at.clear();
                    }
                }
                continue;
            }
            let Some((key, value)) = key_value(l) else { continue };
            let value = if value.len() >= 2 && value.starts_with('"') {
                value.get(1..value.len() - 1).unwrap_or("")
            } else {
                value
            };
            if in_header {
                if key == "name" {
                    hdr_name = value.to_owned();
                } else if key == "position" {
                    hdr_at = value.to_owned();
                }
                continue;
            }
            if !in_spawndata {
                continue;
            }
            // the raw line, not the stripped value: the strip eats the bracket of "[story_object]"
            if key == "000024" {
                cd = if l.contains("[story_object]") { 2 } else { 1 };
                continue;
            }
            match key {
                "000002" => section = value.to_owned(),
                "000003" => name = value.to_owned(),
                "000006" => at = value.to_owned(),
                _ => {}
            }
        }
        let who = if name.is_empty() { hdr_name.clone() } else { name.clone() };
        let pos = if at.is_empty() || at == ZERO_POS { hdr_at } else { at };
        note_spawn_object(&section, &who, &level, &pos, part);
    }
}

/// The module's own dialogs and portraits (`BuildProjectLooks`).
fn project_looks(files: &ProjectFiles, part: &mut ProjectPart) {
    for (_, text) in &files.gameplay {
        if text.contains("<dialog") {
            add_dialogs(text, None, "project", &mut part.looks);
        }
    }
    for (_, text) in &files.textures_descr {
        for t in read_texture_descr(text) {
            part.looks.push((PickerType::Icon, t.id.clone(), String::new(), icon_extra(&t, true)));
        }
    }
}

/// The project's part of the pickers.
#[must_use]
pub fn build_project(files: &ProjectFiles) -> ProjectPart {
    let mut part = ProjectPart::default();
    project_stories(files, &mut part.stories);
    project_restrictors(files, &mut part);
    project_looks(files, &mut part);
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_types() {
        assert_eq!(PickerType::from_name("story_id"), Some(PickerType::Story));
        assert_eq!(PickerType::from_name("task_id"), None);
        assert_eq!(PickerType::LevelChanger.name(), "level_changer");
    }

    #[test]
    fn scans_and_attributes() {
        let ids = scan_ids("<dialogs><dialog id=\"a\"><text>x</text></dialog>\n<dialog\tid=\"b\">", "dialog");
        assert_eq!(ids.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(tag_attr("<texture id=\"x\" xid=\"no\" x=\"5\"", "x"), "5");
        let t = read_texture_descr(
            "<w><!-- <texture id=\"dead\" --><file name=\"ui\\ui_npc\"><texture id=\"face\" x=\"1\" y=\"2\" width=\"3\" height=\"4\"/></file></w>",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t.first().map(|t| icon_extra(t, true)), Some("project:ui\\ui_npc|1|2|3|4".to_owned()));
    }

    #[test]
    fn project_levels_give_restrictors_and_stories() {
        let part_text = "[object_1]\nname = gate_zone\nposition = 10.4, 0, -3.6\n[object_1_spawndata]\n000002 = space_restrictor\n000003 = \n000006 = 0.000000, 0.000000, 0.000000\n000024 = \"[story_object]\nstory_id = gate_guard\n\"\n[object_2]\nname = s1\n";
        let files = ProjectFiles {
            spawn_parts: vec![("rawdata/levels/l01_escape/spawn.part".into(), part_text.into())],
            custom_data: vec![(
                "spawn/custom_data/a.ltx".into(),
                "[story_object]\nstory_id = esc_x ; c\n".into(),
            )],
            ..ProjectFiles::default()
        };
        let part = build_project(&files);
        let p = Pickers::compose(None, &part, &[], &[]);
        assert!(!p.available());
        assert!(p.known(PickerType::Restrictor, "gate_zone"));
        assert_eq!(
            p.entries(PickerType::Restrictor).first().map(|e| e.extra.as_str()),
            Some("l01_escape (10, 0, -4)")
        );
        assert!(p.known(PickerType::Story, "gate_guard"));
        assert!(p.known(PickerType::Story, "esc_x"));
        assert!(p.known(PickerType::Spot, "treasure"));
    }

    #[test]
    fn search_ranks_prefixes_first() {
        let mut part = ProjectPart::default();
        for id in ["b_wolf", "wolf", "a_wolfish"] {
            part.stories.push((PickerType::Story, id.into(), String::new(), "project".into()));
        }
        let p = Pickers::compose(
            None,
            &part,
            &[],
            &[PickerEntry { id: "wolf_scene".into(), name: String::new(), extra: "spawn".into() }],
        );
        let ids: Vec<&str> = p.search(PickerType::Story, "wolf", 10).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["wolf_scene", "wolf", "a_wolfish", "b_wolf"]);
    }
}
