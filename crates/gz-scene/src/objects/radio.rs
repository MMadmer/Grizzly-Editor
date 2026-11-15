//! A radio of the world: what tells one from any other spawn point, as the game tells it
//! (`RADIO.md`, "Which themes are the radio"; `xms_radio.script`, `is_radio_theme`), the radio
//! the Place panel places, and the `[xms_radio]` the bake writes into its custom data.
//!
//! A radio is a physics object whose logic runs the stock `ph_sound` scheme on a radio theme of
//! `plugins\xr_sound.ltx`: `common_radio`, or a section with exactly its tracks (every level radio
//! inherits it unchanged). The logic is the object's custom data, or the file its `[logic] cfg`
//! names, or the file its section's `custom_data` names.

use std::sync::Arc;

use gz_entities::{Entity, Settings};
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

pub use gz_mod::RadioPlaylist;

use crate::model::{ObjData, Object};

/// The Place panel's name of the radio it places (`scene.place kind=spawn section=radio`).
pub const RADIO_CHOOSE_NAME: &str = "$radio";
/// The radio theme every other one copies.
pub const COMMON_THEME: &str = "common_radio";
/// Where the game keeps its radio themes, under `$game_config$`.
pub const THEMES_FILE: &str = "plugins\\xr_sound.ltx";

/// What the game's own level radios are (`all.spawn`: Sidorovich's, the Bar's, ...): a physics
/// object of this section and model, nailed at its root bone. The game has no section of its own
/// for one; its radios carry their logic in their custom data.
pub const RADIO_SECTION: &str = "physic_object";
pub const RADIO_VISUAL: &str = "dynamics\\el_tehnika\\priemnik_gorizont";
pub const RADIO_BONES: &str = "link";
/// The placed radio's logic: the radio theme near the player, the stock noise through a surge
/// (`RADIO.md`, "A radio of your own").
pub const RADIO_LOGIC: &str = "[logic]\r\n\
active = ph_sound@play\r\n\
\r\n\
[ph_sound@play]\r\n\
snd      = common_radio\r\n\
looped   = false\r\n\
random   = true\r\n\
min_idle = 300\r\n\
max_idle = 500\r\n\
volume   = {=dist_to_actor_le(10)} 1, {=dist_to_actor_le(20)} 0.5, 0\r\n\
on_info  = {=surge_started} ph_sound@surge\r\n\
\r\n\
[ph_sound@surge]\r\n\
snd     = broadcast_disrupt\r\n\
random  = true\r\n\
on_info = {=surge_complete} ph_sound@play\r\n";

/// How deep `[logic] cfg` is followed.
const CFG_DEPTH: u32 = 3;

/// A section of an ini text: its name and parents lower case, its lines as `(key, value)`.
struct Section {
    name: String,
    parents: Vec<String>,
    lines: Vec<(String, String)>,
}

/// The sections of an ini text, in order. Comments (`;`) are cut; `#include` lines are skipped.
fn sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(close) = rest.find(']') else { continue };
            let name = rest.get(..close).unwrap_or("").trim().to_ascii_lowercase();
            let parents = rest
                .get(close + 1..)
                .and_then(|t| t.trim().strip_prefix(':'))
                .map(|p| {
                    p.split(',').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect()
                })
                .unwrap_or_default();
            out.push(Section { name, parents, lines: Vec::new() });
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let (k, v) = line.split_once('=').map_or((line, ""), |(k, v)| (k.trim(), v.trim()));
        cur.lines.push((k.to_ascii_lowercase(), v.to_owned()));
    }
    out
}

/// A section's keys, its parents' first, as the game's ini lists them.
fn keys<'a>(name: &str, by: &FxHashMap<&str, &'a Section>, depth: u32, out: &mut Vec<&'a str>) {
    let Some(s) = by.get(name) else { return };
    if depth < 8 {
        for p in &s.parents {
            keys(p, by, depth + 1, out);
        }
    }
    out.extend(s.lines.iter().map(|(k, _)| k.as_str()));
}

/// The radio themes of `plugins\xr_sound.ltx`: `common_radio` and every section whose lines,
/// inherited ones first, are exactly its tracks.
#[must_use]
pub fn radio_themes(text: &str) -> FxHashSet<String> {
    let secs = sections(text);
    let by_name: FxHashMap<&str, &Section> = secs.iter().map(|s| (s.name.as_str(), s)).collect();
    let list = |name: &str| {
        let mut v = Vec::new();
        keys(name, &by_name, 0, &mut v);
        v
    };
    let common = list(COMMON_THEME);
    let mut out = FxHashSet::default();
    out.insert(COMMON_THEME.to_owned());
    if common.is_empty() {
        return out;
    }
    for s in &secs {
        if list(&s.name) == common {
            out.insert(s.name.clone());
        }
    }
    out
}

/// The themes a `snd` value names: a plain theme, or the choices of a condition list
/// (`{=cond} theme_a, theme_b`).
fn snd_themes(v: &str) -> impl Iterator<Item = String> + '_ {
    v.split(',').filter_map(|part| {
        let mut p = part.trim();
        while let Some(rest) = p.strip_prefix('{') {
            p = rest.split_once('}').map_or("", |(_, r)| r).trim();
        }
        let p = p.split_whitespace().next().unwrap_or("");
        (!p.is_empty()).then(|| p.to_ascii_lowercase())
    })
}

/// What the linked game says a radio is, read once per game link. Logic files are read when a
/// spawn point first names them and kept.
pub struct Radios {
    themes: FxHashSet<String>,
    settings: Option<Arc<dyn Settings + Send + Sync>>,
    files: Mutex<FxHashMap<String, bool>>,
}

impl Radios {
    /// Without a game: `common_radio` is the one radio theme, and no file is read.
    #[must_use]
    pub fn unlinked() -> Self {
        Self { themes: radio_themes(""), settings: None, files: Mutex::new(FxHashMap::default()) }
    }

    /// Reads the radio themes of the game's configs. Blocking: a file of the game is read.
    #[must_use]
    pub fn read(settings: Arc<dyn Settings + Send + Sync>) -> Self {
        let text = settings.config_text(THEMES_FILE).map(|t| t.into_owned()).unwrap_or_default();
        Self {
            themes: radio_themes(&text),
            settings: Some(settings),
            files: Mutex::new(FxHashMap::default()),
        }
    }

    /// Whether `theme` is radio music.
    #[must_use]
    pub fn is_radio_theme(&self, theme: &str) -> bool {
        self.themes.contains(&theme.trim().to_ascii_lowercase())
    }

    /// Whether a logic text runs `ph_sound` on a radio theme, `[logic] cfg` followed; `None`
    /// for a text that holds no logic at all. Blocking the first time a `cfg` file is named.
    #[must_use]
    pub fn logic_is_radio(&self, text: &str) -> Option<bool> {
        self.judge(text, 0)
    }

    fn judge(&self, text: &str, depth: u32) -> Option<bool> {
        let secs = sections(text);
        if secs.is_empty() {
            return None;
        }
        let runs = secs.iter().any(|s| {
            (s.name == "ph_sound" || s.name.starts_with("ph_sound@"))
                && s.lines.iter().any(|(k, v)| k == "snd" && snd_themes(v).any(|t| self.themes.contains(&t)))
        });
        if runs {
            return Some(true);
        }
        let cfg = secs
            .iter()
            .find(|s| s.name == "logic")
            .and_then(|s| s.lines.iter().find(|(k, _)| k == "cfg"))
            .map(|(_, v)| v.clone());
        match cfg {
            Some(path) if depth < CFG_DEPTH => Some(self.file_is_radio(&path, depth + 1)),
            _ => Some(false),
        }
    }

    /// Whether a file of the game's configs is a radio's logic, kept once read.
    fn file_is_radio(&self, path: &str, depth: u32) -> bool {
        let key = path.trim().replace('/', "\\").to_ascii_lowercase();
        if key.is_empty() {
            return false;
        }
        if let Some(v) = self.files.lock().get(&key) {
            return *v;
        }
        let text = self.settings.as_ref().and_then(|s| s.config_text(&key).map(|t| t.into_owned()));
        let v = text.is_some_and(|t| self.judge(&t, depth) == Some(true));
        self.files.lock().insert(key, v);
        v
    }

    /// Whether an entity is a radio: its own logic, else the logic its section's `custom_data`
    /// names. Blocking the first time a file is named.
    #[must_use]
    pub fn is_radio(&self, e: &Entity) -> bool {
        if e.object().and_then(|o| o.physic()).is_none() {
            return false;
        }
        if let Some(v) = e.custom_data().and_then(|t| self.logic_is_radio(t)) {
            return v;
        }
        let section = e.section();
        let file = self
            .settings
            .as_ref()
            .filter(|s| s.section_exist(section))
            .and_then(|s| s.r_string(section, "custom_data").map(str::to_owned));
        file.is_some_and(|f| self.file_is_radio(&f, 1))
    }
}

/// What the Radio rows, the audit and the bake read besides the scene: the game's radios and the
/// module's playlists.
#[derive(Clone)]
pub struct RadioCx {
    pub radios: Arc<Radios>,
    /// The module's id (`[module] id`): what a playlist of its own is keyed by.
    pub module: String,
    pub playlists: Arc<[gz_mod::RadioPlaylist]>,
}

/// One choice of the Plays row: what is written, what it is called, whether the name is data
/// (a playlist's title) rather than the editor's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaysChoice {
    pub value: String,
    pub label: String,
    pub data: bool,
}

/// The label of the Plays row's first choice: nothing written.
pub const PLAYER_CHOICE: &str = "As the Player Chose";
pub const GAME_MUSIC: &str = "The Game's Music";
pub const SILENCE: &str = "Silence";

impl RadioCx {
    /// No game, no module.
    #[must_use]
    pub fn unlinked() -> Self {
        Self { radios: Arc::new(Radios::unlinked()), module: String::new(), playlists: Arc::from(Vec::new()) }
    }

    /// Whether a spawn point is a radio of the world.
    #[must_use]
    pub fn is_radio(&self, o: &Object) -> bool {
        let ObjData::Spawn(d) = &o.data else { return false };
        d.entity
            .as_ref()
            .and_then(crate::objects::SpawnEntity::entity)
            .is_some_and(|e| self.radios.is_radio(e))
    }

    /// The Plays row's list: the player's choice, every playlist of the module by its title, the
    /// game's music, silence; a value the list lacks is kept and shown as written.
    #[must_use]
    pub fn choices(&self, current: &str) -> Vec<PlaysChoice> {
        let own = |v: &str, l: &str| PlaysChoice { value: v.to_owned(), label: l.to_owned(), data: false };
        let mut out = vec![own("", PLAYER_CHOICE)];
        for p in self.playlists.iter() {
            let label = if p.title.is_empty() { p.id.clone() } else { p.title.clone() };
            out.push(PlaysChoice { value: p.id.clone(), label, data: true });
        }
        out.push(own(gz_mod::radio::GAME, GAME_MUSIC));
        out.push(own(gz_mod::radio::OFF, SILENCE));
        let cur = current.trim();
        if !cur.is_empty() && !out.iter().any(|c| self.same(&c.value, cur)) {
            out.push(PlaysChoice { value: cur.to_owned(), label: cur.to_owned(), data: true });
        }
        out
    }

    /// Whether two bindings say the same: an id alone and `<this module>.<id>` are one playlist.
    #[must_use]
    pub fn same(&self, a: &str, b: &str) -> bool {
        use gz_mod::radio::{Binding, binding};
        match (binding(a, &self.module), binding(b, &self.module)) {
            (Ok(Binding::Own(x)), Ok(Binding::Own(y))) => x.eq_ignore_ascii_case(y),
            (Ok(x), Ok(y)) => x == y,
            _ => a.trim() == b.trim(),
        }
    }

    /// The value a text names: a choice's value or label, or any other binding as written; an
    /// error for a text that is no binding.
    pub fn pick(&self, text: &str) -> Result<String, String> {
        let t = text.trim();
        let list = self.choices("");
        if let Some(c) = list
            .iter()
            .find(|c| c.label.eq_ignore_ascii_case(t) || (!c.value.is_empty() && self.same(&c.value, t)))
        {
            return Ok(c.value.clone());
        }
        if t.is_empty() {
            return Ok(String::new());
        }
        gz_mod::radio::binding(t, &self.module).map(|_| t.to_owned())
    }

    /// The id of a playlist of this module a binding names that `mod.ltx` lacks.
    #[must_use]
    pub fn missing<'a>(&self, plays: &'a str) -> Option<&'a str> {
        match gz_mod::radio::binding(plays, &self.module) {
            Ok(gz_mod::radio::Binding::Own(id)) if !gz_mod::radio::declares(&self.playlists, id) => Some(id),
            _ => None,
        }
    }

    /// `[xms_radio]` with the binding as the game reads it, or why the binding is no playlist.
    pub fn section(&self, plays: &str) -> Result<String, String> {
        let b = gz_mod::radio::binding(plays, &self.module)?;
        if matches!(b, gz_mod::radio::Binding::Own(_)) && self.module.is_empty() {
            return Err("the module has no id in mod.ltx: a playlist of its own cannot be named".to_owned());
        }
        Ok(format!("[xms_radio]\r\nplaylist = {}\r\n", b.key(&self.module)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: &str = "\u{feff}; Radio mod\n[common_radio]\nradio\\01\\music_01\nradio\\01\\music_02\n\n[scientific_radio]\nradio\\02\\marsh_beep_1\n\n[bar_visitors_radio]:common_radio\n\n[yan_saharov_radio]:scientific_radio\n\n[static]\nsoundtrack\\static\\static_01\n";

    #[test]
    fn the_radio_themes_are_common_radio_and_its_copies() {
        let t = radio_themes(THEMES);
        assert!(t.contains("common_radio") && t.contains("bar_visitors_radio"));
        assert!(!t.contains("scientific_radio") && !t.contains("yan_saharov_radio") && !t.contains("static"));
    }

    #[test]
    fn a_logic_is_a_radio_when_a_ph_sound_section_plays_a_radio_theme() {
        let r =
            Radios { themes: radio_themes(THEMES), settings: None, files: Mutex::new(FxHashMap::default()) };
        assert_eq!(r.logic_is_radio(RADIO_LOGIC), Some(true));
        let stock = "[logic]\nactive = {=is_laststand} ph_sound@zombie, ph_sound@normal\n[ph_sound@normal]\nsnd = bar_visitors_radio\n";
        assert_eq!(r.logic_is_radio(stock), Some(true));
        let noise = "[logic]\nactive = ph_sound@n\n[ph_sound@n]\nsnd = static\n";
        assert_eq!(r.logic_is_radio(noise), Some(false));
        let walker = "[logic]\nactive = walker@a\n[walker@a]\nsnd = common_radio\n";
        assert_eq!(r.logic_is_radio(walker), Some(false));
        assert_eq!(r.logic_is_radio(""), None);
        assert_eq!(r.logic_is_radio("[logic]\ncfg = scripts\\x.ltx\n"), Some(false));
        assert_eq!(
            snd_themes("{=a} common_radio, {+b} static").collect::<Vec<_>>(),
            ["common_radio", "static"]
        );
    }

    #[test]
    fn the_plays_row_lists_the_playlists_and_keeps_a_stray_value() {
        let cx = RadioCx {
            radios: Arc::new(Radios::unlinked()),
            module: "my_mod".into(),
            playlists: Arc::from(vec![gz_mod::RadioPlaylist {
                id: "road".into(),
                title: "Road Songs".into(),
            }]),
        };
        let labels: Vec<String> = cx.choices("").iter().map(|c| c.label.clone()).collect();
        assert_eq!(labels, [PLAYER_CHOICE, "Road Songs", GAME_MUSIC, SILENCE]);
        assert_eq!(cx.choices("gone").last().map(|c| c.value.as_str()), Some("gone"));
        assert_eq!(cx.choices("my_mod.road").len(), 4);
        assert_eq!(cx.pick("Road Songs").as_deref(), Ok("road"));
        assert_eq!(cx.pick("my_mod.road").as_deref(), Ok("road"));
        assert_eq!(cx.pick("The Game's Music").as_deref(), Ok("game"));
        assert_eq!(cx.pick("other.x").as_deref(), Ok("other.x"));
        assert!(cx.pick("no such thing").is_err());
        assert_eq!(cx.missing("gone"), Some("gone"));
        assert_eq!(cx.missing("road"), None);
        assert_eq!(cx.missing("other.x"), None);
        assert_eq!(cx.section("road").as_deref(), Ok("[xms_radio]\r\nplaylist = my_mod.road\r\n"));
        assert_eq!(cx.section("off").as_deref(), Ok("[xms_radio]\r\nplaylist = off\r\n"));
    }
}
