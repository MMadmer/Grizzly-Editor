//! The model of `[screens]`, `[screen.<id>]`, `[track.<id>]`, `[playlist.<id>]` and `[radio]`:
//! read off the manifest document, written back by the screens' edits.

use gz_mod::doc::Item;
use gz_mod::{Edit, ManifestDoc};

/// The surfaces a screen shows on or a track plays on.
pub const LOADING: u32 = 1;
pub const MENU: u32 = 2;
pub const PAUSE: u32 = 4;
pub const ALL: u32 = 7;
/// A track's `play`: the radios of the world shuffle it among the game's own radio music.
pub const RADIO: u32 = 8;

/// What a loading screen hides.
pub const HIDE_BAR: u32 = 1;
pub const HIDE_PERCENT: u32 = 2;
pub const HIDE_STAGE: u32 = 4;
pub const HIDE_TIP: u32 = 8;

/// How a layer's picture fills its rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
}

impl Fit {
    /// `contain`, `stretch`, anything else `cover`.
    #[must_use]
    pub fn parse(v: &str) -> Self {
        match v {
            "contain" => Self::Contain,
            "stretch" => Self::Stretch,
            _ => Self::Cover,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Stretch => "stretch",
        }
    }
}

/// One layer: one file.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Project-relative, `\` separators; empty: nothing put on it yet.
    pub file: String,
    /// A shader's own picture.
    pub texture: String,
    pub fit: Fit,
    pub opacity: f32,
    /// Per cent of the screen.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub sound: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            file: String::new(),
            texture: String::new(),
            fit: Fit::Cover,
            opacity: 1.0,
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            sound: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub id: String,
    pub show: u32,
    pub levels: Vec<String>,
    pub modes: Vec<String>,
    pub weight: f32,
    /// `any`, `none`, `video` or `tracks`.
    pub music: String,
    /// The tracks of `music = tracks`.
    pub tracks: Vec<String>,
    pub hide: u32,
    /// Bottom first.
    pub layers: Vec<Layer>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            id: String::new(),
            show: ALL,
            levels: Vec::new(),
            modes: Vec::new(),
            weight: 1.0,
            music: "any".to_owned(),
            tracks: Vec::new(),
            hide: 0,
            layers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: String,
    pub file: String,
    /// The shuffles it joins: the surfaces' and [`RADIO`].
    pub play: u32,
    pub weight: f32,
    pub volume: f32,
    /// Not played while the game's streamer mode is on.
    pub copyrighted: bool,
    /// What plays instead in the streamer mode: a track id of the module or a sound name;
    /// empty: nothing.
    pub streamer: String,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            id: String::new(),
            file: String::new(),
            play: 0,
            weight: 1.0,
            volume: 1.0,
            copyrighted: false,
            streamer: String::new(),
        }
    }
}

/// The order a playlist plays its tracks in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Every track once a pass, in a new order each pass.
    #[default]
    Shuffle,
    /// In the playlist's order, round and round.
    List,
}

impl Order {
    /// `list`; anything else is `shuffle`, as the game reads it.
    #[must_use]
    pub fn parse(v: &str) -> Self {
        if v.trim().eq_ignore_ascii_case("list") { Self::List } else { Self::Shuffle }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Shuffle => "shuffle",
            Self::List => "list",
        }
    }
}

/// A station of the radio: a named list of the module's tracks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub id: String,
    /// What the player reads: text or a string table id; empty: the id.
    pub title: String,
    /// Track ids, in order.
    pub tracks: Vec<String>,
    pub order: Order,
}

/// The module's screens and music. `[0]` is the loading screen, `[1]` the menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screens {
    pub only_screens: [bool; 2],
    pub only_music: [bool; 2],
    pub next_track: [bool; 2],
    pub screens: Vec<Screen>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    /// `[radio] music = only`: this module's radio tracks are the radios' whole common music.
    pub only_radio: bool,
}

/// C `atof`: the longest number at the start of the text (after white space), 0 without one.
#[must_use]
pub fn atof(s: &str) -> f64 {
    scan_number(s).map_or(0.0, |(v, _)| v)
}

/// A C `strtod` prefix: the number and the bytes it took (white space included).
fn scan_number(s: &str) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while b.get(i).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)) {
        i += 1;
    }
    let start = i;
    if matches!(b.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_from = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - digits_from;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let frac = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        digits += i - frac;
    }
    if digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = j;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > exp {
            i = j;
        }
    }
    s.get(start..i).and_then(|t| t.parse::<f64>().ok()).map(|v| (v, i))
}

/// `sscanf("%f %f %f %f")`: as many numbers as read from the start, up to `n`.
#[must_use]
pub fn scan_floats(s: &str, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    let mut rest = s;
    while out.len() < n {
        let Some((v, used)) = scan_number(rest) else { break };
        out.push(v as f32);
        rest = rest.get(used..).unwrap_or("");
    }
    out
}

/// Pieces between `sep`, trimmed, empty ones dropped.
#[must_use]
pub fn split(s: &str, sep: char) -> Vec<String> {
    s.split(sep)
        .map(|p| p.trim_matches([' ', '\t', '\r', '\n']))
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The surfaces of a file's `show` / `play`: names separated by `,`.
#[must_use]
pub fn surfaces(value: &str) -> u32 {
    split(value, ',').iter().fold(0, |m, n| {
        m | match n.as_str() {
            "loading" => LOADING,
            "menu" => MENU,
            "pause" => PAUSE,
            _ => 0,
        }
    })
}

/// Where a track plays (`play`): the surfaces and the radio.
#[must_use]
pub fn plays(value: &str) -> u32 {
    split(value, ',').iter().fold(0, |m, n| {
        m | match n.as_str() {
            "radio" => RADIO,
            other => surfaces(other),
        }
    })
}

/// `loading, menu, pause, radio` as the file writes them.
#[must_use]
pub fn surfaces_text(mask: u32) -> String {
    let names: Vec<&str> = [(LOADING, "loading"), (MENU, "menu"), (PAUSE, "pause"), (RADIO, "radio")]
        .into_iter()
        .filter(|(b, _)| mask & b != 0)
        .map(|(_, n)| n)
        .collect();
    names.join(", ")
}

/// `%g` of a float, as the file writes numbers.
#[must_use]
pub fn number(v: f32) -> String {
    gz_luadata::format_g(f64::from(v), 6)
}

/// An id made of a name: lower case letters, digits and `_`.
#[must_use]
pub fn sanitize(name: &str) -> String {
    let out: String = name
        .bytes()
        .map(|c| match c {
            b'A'..=b'Z' => char::from(c - b'A' + b'a'),
            b'a'..=b'z' | b'0'..=b'9' | b'_' => char::from(c),
            _ => '_',
        })
        .collect();
    if out.is_empty() { "item".to_owned() } else { out }
}

fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut id = sanitize(base);
    let mut n = 2u32;
    while taken(&id) {
        id = format!("{}_{n}", sanitize(base));
        n += 1;
    }
    id
}

fn parse_screen_key(s: &mut Screen, key: &str, val: &str) {
    match key {
        "layer" => s.layers.push(Layer { file: val.to_owned(), ..Layer::default() }),
        "show" => s.show = surfaces(val),
        "levels" => s.levels = split(val, ','),
        "modes" => s.modes = split(val, ','),
        "weight" => s.weight = atof(val) as f32,
        "music" => {
            s.tracks.clear();
            if val.is_empty() || val == "any" {
                s.music = "any".to_owned();
            } else if val == "none" || val == "video" {
                s.music = val.to_owned();
            } else {
                s.music = "tracks".to_owned();
                s.tracks = split(val, ',');
            }
        }
        "hide" => {
            s.hide = split(val, ',').iter().fold(0, |m, n| {
                m | match n.as_str() {
                    "bar" => HIDE_BAR,
                    "percent" => HIDE_PERCENT,
                    "stage" => HIDE_STAGE,
                    "tip" => HIDE_TIP,
                    _ => 0,
                }
            });
        }
        _ => {
            let Some(layer) = s.layers.last_mut() else { return };
            match key {
                "fit" => layer.fit = Fit::parse(val),
                "opacity" => layer.opacity = atof(val) as f32,
                "rect" => {
                    let v = scan_floats(val, 4);
                    for (slot, n) in
                        [&mut layer.x, &mut layer.y, &mut layer.w, &mut layer.h].into_iter().zip(v)
                    {
                        *slot = n;
                    }
                }
                "texture" => layer.texture = val.to_owned(),
                "sound" => layer.sound = !matches!(val, "false" | "0" | "no" | "off"),
                _ => {}
            }
        }
    }
}

fn parse_track_key(t: &mut Track, key: &str, val: &str) {
    match key {
        "file" => t.file = val.to_owned(),
        "play" => t.play = plays(val),
        "weight" => t.weight = atof(val) as f32,
        "volume" => t.volume = atof(val) as f32,
        "copyrighted" => t.copyrighted = matches!(val, "true" | "1" | "yes" | "on"),
        "streamer" => t.streamer = val.to_owned(),
        _ => {}
    }
}

fn parse_playlist_key(p: &mut Playlist, key: &str, val: &str) {
    match key {
        "title" => p.title = gz_mod::text::text_value(val),
        "tracks" => p.tracks = split(val, ','),
        "order" => p.order = Order::parse(val),
        _ => {}
    }
}

fn parse_radio_key(s: &mut Screens, key: &str, val: &str) {
    if key == "music" {
        s.only_radio = val == "only";
    }
}

/// `key = value` with the key padded to 17, as a screens row is written.
fn row(key: &str, value: &str) -> String {
    format!("{key:<17} = {value}")
}

#[derive(Clone, Copy)]
enum Sec {
    Other,
    Screens,
    Screen(usize),
    Track(usize),
    Playlist(usize),
    Radio,
}

impl Screens {
    /// The model of a manifest document: a section header read lower case,
    /// every `[screen.<id>]` header a screen of its own, keys and values as written.
    #[must_use]
    pub fn parse(doc: &ManifestDoc) -> Self {
        let mut s = Screens::default();
        let mut sec = Sec::Other;
        for item in doc.items() {
            match item {
                Item::Section(name) => {
                    let sn = name.to_ascii_lowercase();
                    sec = if sn == "screens" {
                        Sec::Screens
                    } else if let Some(id) = sn.strip_prefix("screen.").filter(|id| !id.is_empty()) {
                        s.screens.push(Screen { id: id.to_owned(), ..Screen::default() });
                        Sec::Screen(s.screens.len() - 1)
                    } else if let Some(id) = sn.strip_prefix("track.").filter(|id| !id.is_empty()) {
                        s.tracks.push(Track { id: id.to_owned(), ..Track::default() });
                        Sec::Track(s.tracks.len() - 1)
                    } else if let Some(id) = sn.strip_prefix("playlist.").filter(|id| !id.is_empty()) {
                        s.playlists.push(Playlist { id: id.to_owned(), ..Playlist::default() });
                        Sec::Playlist(s.playlists.len() - 1)
                    } else if sn == "radio" {
                        Sec::Radio
                    } else {
                        Sec::Other
                    };
                }
                Item::Entry { key, value, .. } => match sec {
                    Sec::Screens => {
                        let (only, next) = (value == "only", value == "next");
                        match key {
                            "loading_screens" => s.only_screens[0] = only,
                            "menu_screens" => s.only_screens[1] = only,
                            "loading_music" => s.only_music[0] = only,
                            "menu_music" => s.only_music[1] = only,
                            "loading_music_end" => s.next_track[0] = next,
                            "menu_music_end" => s.next_track[1] = next,
                            _ => {}
                        }
                    }
                    Sec::Screen(i) => {
                        if let Some(screen) = s.screens.get_mut(i) {
                            parse_screen_key(screen, key, value);
                        }
                    }
                    Sec::Track(i) => {
                        if let Some(track) = s.tracks.get_mut(i) {
                            parse_track_key(track, key, value);
                        }
                    }
                    Sec::Playlist(i) => {
                        if let Some(p) = s.playlists.get_mut(i) {
                            parse_playlist_key(p, key, value);
                        }
                    }
                    Sec::Radio => parse_radio_key(&mut s, key, value),
                    Sec::Other => {}
                },
                Item::Other => {}
            }
        }
        s
    }

    /// The lines of `[screens]`: empty when every policy is the default.
    #[must_use]
    pub fn policy_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.only_screens[0] {
            out.push(row("loading_screens", "only"));
        }
        if self.only_music[0] {
            out.push(row("loading_music", "only"));
        }
        if self.only_screens[1] {
            out.push(row("menu_screens", "only"));
        }
        if self.only_music[1] {
            out.push(row("menu_music", "only"));
        }
        if self.next_track[0] {
            out.push(row("loading_music_end", "next"));
        }
        if self.next_track[1] {
            out.push(row("menu_music_end", "next"));
        }
        out
    }

    /// The lines of a screen's section.
    #[must_use]
    pub fn screen_lines(d: &Screen) -> Vec<String> {
        let mut out = vec![row("show", &surfaces_text(d.show))];
        if !d.levels.is_empty() {
            out.push(row("levels", &d.levels.join(", ")));
        }
        if !d.modes.is_empty() {
            out.push(row("modes", &d.modes.join(", ")));
        }
        if d.weight != 1.0 {
            out.push(row("weight", &number(d.weight)));
        }
        if d.music == "tracks" {
            out.push(row("music", &d.tracks.join(", ")));
        } else if d.music != "any" {
            out.push(row("music", &d.music));
        }
        if d.hide != 0 {
            let names: Vec<&str> =
                [(HIDE_BAR, "bar"), (HIDE_PERCENT, "percent"), (HIDE_STAGE, "stage"), (HIDE_TIP, "tip")]
                    .into_iter()
                    .filter(|(b, _)| d.hide & b != 0)
                    .map(|(_, n)| n)
                    .collect();
            out.push(row("hide", &names.join(", ")));
        }
        for l in &d.layers {
            out.push(row("layer", &l.file));
            match l.fit {
                Fit::Contain => out.push(row("fit", "contain")),
                Fit::Stretch => out.push(row("fit", "stretch")),
                Fit::Cover => {}
            }
            if l.opacity != 1.0 {
                out.push(row("opacity", &number(l.opacity)));
            }
            if l.x != 0.0 || l.y != 0.0 || l.w != 100.0 || l.h != 100.0 {
                out.push(row(
                    "rect",
                    &format!("{} {} {} {}", number(l.x), number(l.y), number(l.w), number(l.h)),
                ));
            }
            if !l.texture.is_empty() {
                out.push(row("texture", &l.texture));
            }
            if !l.sound {
                out.push(row("sound", "false"));
            }
        }
        out
    }

    /// The lines of a track's section.
    #[must_use]
    pub fn track_lines(t: &Track) -> Vec<String> {
        let mut out = vec![row("file", &t.file)];
        if t.play != 0 {
            out.push(row("play", &surfaces_text(t.play)));
        }
        if t.weight != 1.0 {
            out.push(row("weight", &number(t.weight)));
        }
        if t.volume != 1.0 {
            out.push(row("volume", &number(t.volume)));
        }
        if t.copyrighted {
            out.push(row("copyrighted", "true"));
        }
        if !t.streamer.is_empty() {
            out.push(row("streamer", &t.streamer));
        }
        out
    }

    /// The lines of a playlist's section.
    #[must_use]
    pub fn playlist_lines(p: &Playlist) -> Vec<String> {
        let mut out = Vec::new();
        if !p.title.is_empty() {
            out.push(row("title", &gz_mod::text::display_value(&p.title, false)));
        }
        out.push(row("tracks", &p.tracks.join(", ")));
        if p.order == Order::List {
            out.push(row("order", "list"));
        }
        out
    }

    /// The lines of `[radio]`: empty when the radio mixes in.
    #[must_use]
    pub fn radio_lines(&self) -> Vec<String> {
        if self.only_radio { vec![row("music", "only")] } else { Vec::new() }
    }

    /// The edits that make `doc` hold this model. Sections that keep their names and order are
    /// rewritten in place (the author's comments around them stay); when screens or tracks were
    /// added, removed, renamed or reordered, their sections are written anew in the model's
    /// order, after the rest of the file.
    #[must_use]
    pub fn edits(&self, doc: &ManifestDoc) -> Vec<Edit> {
        let mut out =
            vec![Edit::lines("screens", self.policy_lines()), Edit::lines("radio", self.radio_lines())];
        let screens: Vec<(String, Vec<String>)> =
            self.screens.iter().map(|d| (format!("screen.{}", d.id), Self::screen_lines(d))).collect();
        let tracks: Vec<(String, Vec<String>)> =
            self.tracks.iter().map(|t| (format!("track.{}", t.id), Self::track_lines(t))).collect();
        let playlists: Vec<(String, Vec<String>)> =
            self.playlists.iter().map(|p| (format!("playlist.{}", p.id), Self::playlist_lines(p))).collect();
        for (prefix, want) in [("screen.", screens), ("track.", tracks), ("playlist.", playlists)] {
            let have: Vec<String> = doc
                .sections_with_prefix(prefix)
                .into_iter()
                .filter(|n| n.len() > prefix.len())
                .map(str::to_ascii_lowercase)
                .collect();
            let same = have.len() == want.len()
                && have.iter().zip(&want).all(|(h, (w, _))| *h == w.to_ascii_lowercase());
            if !same {
                out.extend(have.into_iter().map(|section| Edit::RemoveSection { section }));
            }
            out.extend(want.into_iter().map(|(section, lines)| Edit::SetLines { section, lines }));
        }
        out
    }

    #[must_use]
    pub fn screen(&self, id: &str) -> Option<&Screen> {
        self.screens.iter().find(|d| d.id == id)
    }

    pub fn screen_mut(&mut self, id: &str) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|d| d.id == id)
    }

    #[must_use]
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|d| d.id == id)
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|d| d.id == id)
    }

    /// A screen id made of `base` that no screen has.
    #[must_use]
    pub fn new_screen_id(&self, base: &str) -> String {
        unique_id(base, |id| self.screen(id).is_some())
    }

    /// A track id made of `base` that no track has.
    #[must_use]
    pub fn new_track_id(&self, base: &str) -> String {
        unique_id(base, |id| self.track(id).is_some())
    }

    #[must_use]
    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    /// A playlist id made of `base` that no playlist has.
    #[must_use]
    pub fn new_playlist_id(&self, base: &str) -> String {
        unique_id(base, |id| self.playlist(id).is_some())
    }

    /// Takes a track out, and out of every screen and playlist that names it, and out of every
    /// track that plays it instead.
    pub fn remove_track(&mut self, id: &str) {
        self.tracks.retain(|t| t.id != id);
        for p in &mut self.playlists {
            p.tracks.retain(|t| t != id);
        }
        for t in &mut self.tracks {
            if t.streamer == id {
                t.streamer.clear();
            }
        }
        for d in &mut self.screens {
            d.tracks.retain(|t| t != id);
            if d.music == "tracks" && d.tracks.is_empty() {
                "any".clone_into(&mut d.music);
            }
        }
    }
}
