//! The sound asset (`*.sndasset`, `snd = 1`): a sound and the parameters the game plays it
//! with - what a sound's `.thm` says, as named keys.

use gz_fmt_sound::SoundThm;
use gz_luadata::{Table, Value};

use crate::fields::{self, SOUND};
use crate::read::{self, Cx};

/// A sound asset as its file says; `None` is a key left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundAsset {
    pub source: Option<String>,
    pub name: Option<String>,
    pub min_distance: Option<f64>,
    pub max_distance: Option<f64>,
    pub volume: Option<f64>,
    pub ai_distance: Option<f64>,
    /// [`fields::SOUND_TYPES`].
    pub ty: Option<u32>,
    pub quality: Option<f64>,
    /// Not played while the game's streamer mode is on.
    pub copyrighted: Option<bool>,
    /// The game name of what plays instead while the streamer mode is on.
    pub streamer: Option<String>,
}

impl SoundAsset {
    #[must_use]
    pub fn from_value(v: &Value) -> (Self, Vec<read::Issue>) {
        let mut cx = Cx::default();
        let s = SoundAsset {
            source: read::text(&mut cx, v, "source"),
            name: read::text(&mut cx, v, "name"),
            min_distance: read::number(&mut cx, v, SOUND, "min_distance"),
            max_distance: read::number(&mut cx, v, SOUND, "max_distance"),
            volume: read::number(&mut cx, v, SOUND, "volume"),
            ai_distance: read::number(&mut cx, v, SOUND, "ai_distance"),
            ty: read::word_set(&mut cx, v, SOUND, "type"),
            quality: read::number(&mut cx, v, SOUND, "quality"),
            copyrighted: read::boolean(&mut cx, v, "copyrighted"),
            streamer: read::text(&mut cx, v, "streamer").filter(|s| !s.trim().is_empty()),
        };
        (s, cx.issues)
    }

    /// Whether the streamer mode keeps it from playing.
    #[must_use]
    pub fn is_copyrighted(&self) -> bool {
        self.copyrighted == Some(true)
    }

    /// What the game plays the sound with, and its encoding quality.
    #[must_use]
    pub fn to_thm(&self) -> SoundThm {
        let d = SoundThm::default();
        let max = self.max_distance.map_or(d.max_dist, |x| x as f32);
        SoundThm {
            quality: self.quality.map_or(d.quality, |x| x as f32),
            min_dist: self.min_distance.map_or(d.min_dist, |x| x as f32),
            max_dist: max,
            max_ai_dist: self.ai_distance.map_or(max, |x| x as f32),
            base_volume: self.volume.map_or(d.base_volume, |x| x as f32),
            game_type: self.ty.unwrap_or(d.game_type),
        }
    }

    /// The asset that says what a sound's description says, writing what differs from the
    /// defaults.
    #[must_use]
    pub fn from_thm(t: &SoundThm) -> Self {
        let d = SoundThm::default();
        let some = |v: f32, def: f32| (v != def).then_some(f64::from(v));
        SoundAsset {
            source: None,
            name: None,
            min_distance: some(t.min_dist, d.min_dist),
            max_distance: some(t.max_dist, d.max_dist),
            volume: some(t.base_volume, d.base_volume),
            ai_distance: some(t.max_ai_dist, t.max_dist),
            ty: (t.game_type != d.game_type).then_some(t.game_type),
            quality: some(t.quality, d.quality),
            copyrighted: None,
            streamer: None,
        }
    }

    #[must_use]
    pub fn to_table(&self) -> Table {
        let mut top = Table::new();
        top.put("snd", 1);
        for f in SOUND {
            let v: Option<Value> = match f.key {
                "source" => self.source.clone().map(Value::from),
                "name" => self.name.clone().map(Value::from),
                "min_distance" => self.min_distance.map(Value::from),
                "max_distance" => self.max_distance.map(Value::from),
                "volume" => self.volume.map(Value::from),
                "ai_distance" => self.ai_distance.map(Value::from),
                // one word where one stands for it, else the words of its bits
                "type" => self.ty.map(|n| match fields::words_of(fields::SOUND_TYPES, n).as_slice() {
                    [one] => Value::from(*one),
                    many => Value::Table(Table {
                        arr: many.iter().map(|w| Value::from(*w)).collect(),
                        fields: Vec::new(),
                    }),
                }),
                "quality" => self.quality.map(Value::from),
                "copyrighted" => self.copyrighted.map(Value::from),
                "streamer" => self.streamer.clone().map(Value::from),
                _ => None,
            };
            if let Some(v) = v {
                top.put(f.key, v);
            }
        }
        top
    }
}

// ---- the streamer mode -------------------------------------------------------------------------

/// The key of a sound asset that marks it copyrighted: the one place the mark is said.
pub const MARK_KEY: &str = "copyrighted";
/// The key that names what plays in a copyrighted sound's place in the streamer mode.
pub const REPLACEMENT_KEY: &str = "streamer";

/// Where the game keeps its own list of copyrighted sounds (the game's `docs/dead-air/RADIO.md`):
/// `[copyrighted]`, one line per sound name or mask with `*`, optionally `= <replacement>`.
pub const RIGHTS_FILE: &str = "configs\\dead_air_x64_sound_rights.ltx";

/// One line of the game's own list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Right {
    /// A name or a mask with `*`, as [`read::name_key`] keys it.
    pub mask: String,
    /// What plays in its place in the streamer mode, as written; empty: nothing.
    pub instead: String,
}

/// The lines of the game's own list, each mask once (the first line of it). A game without the
/// file has an empty list.
#[must_use]
pub fn parse_rights(text: &str) -> Vec<Right> {
    let mut out: Vec<Right> = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or("").trim();
            inside = name.eq_ignore_ascii_case("copyrighted");
            continue;
        }
        if !inside {
            continue;
        }
        let (mask, instead) = line.split_once('=').unwrap_or((line, ""));
        let mask = read::name_key(mask);
        if !mask.is_empty() && !out.iter().any(|r| r.mask == mask) {
            out.push(Right { mask, instead: instead.trim().trim_matches('"').to_owned() });
        }
    }
    out
}

/// Whether a name (already a [`read::name_key`]) matches a mask, `*` standing for any run of
/// characters.
#[must_use]
pub fn mask_matches(mask: &str, name: &str) -> bool {
    let parts: Vec<&str> = mask.split('*').collect();
    let [first, middle @ .., last] = parts.as_slice() else { return mask == name };
    let Some(mut rest) = name.strip_prefix(first) else { return false };
    for p in middle {
        match rest.find(p) {
            Some(at) => rest = rest.get(at + p.len()..).unwrap_or(""),
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// The line of the game's own list that names a sound: the first that matches.
#[must_use]
pub fn right_of<'a>(rights: &'a [Right], name: &str) -> Option<&'a Right> {
    let key = read::name_key(name);
    if key.is_empty() {
        return None;
    }
    rights.iter().find(|r| mask_matches(&r.mask, &key))
}

/// Whether the game's own list names a sound.
#[must_use]
pub fn listed(rights: &[Right], name: &str) -> bool {
    right_of(rights, name).is_some()
}

/// What a sound is to the streamer mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// It plays in the streamer mode.
    Free,
    /// The streamer mode keeps it from playing.
    Copyrighted,
    /// Neither the module nor the linked game has it.
    Missing,
}

/// A sound by its game name: one of the module's sound assets (by what it says of itself), else
/// one of the linked game's (by the game's own list). Without a linked game a name the module
/// lacks is taken as free: nothing says otherwise. Blocking: a sound asset of the module is read.
#[must_use]
pub fn mark(
    root: &std::path::Path,
    index: &crate::index::Index,
    game: &crate::service::GameLists,
    name: &str,
) -> Mark {
    use gz_project::asset_kind::MediaKind;
    if let Some(e) = index.by_name(MediaKind::Sound, name) {
        let copyrighted =
            crate::resolve::sound(root, e.path.as_str()).is_ok_and(|(a, _, _)| a.is_copyrighted());
        return if copyrighted { Mark::Copyrighted } else { Mark::Free };
    }
    if !game.linked {
        return Mark::Free;
    }
    if !game.has_sound(name) {
        return Mark::Missing;
    }
    if listed(&game.rights, name) { Mark::Copyrighted } else { Mark::Free }
}

/// What a sound is to the streamer mode, read where the game reads it: the module's sound asset
/// that publishes the name (what it says of itself), else the linked game's own list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub mark: Mark,
    /// What plays in its place in the streamer mode; empty: it is left out (or it is free).
    pub instead: String,
    /// The module's sound asset that publishes it, project-relative; `None` for a sound of the
    /// game (or nobody's).
    pub asset: Option<String>,
}

/// [`Facts`] of a sound by its game name. Blocking: a sound asset of the module is read.
#[must_use]
pub fn facts(
    root: &std::path::Path,
    index: &crate::index::Index,
    game: &crate::service::GameLists,
    name: &str,
) -> Facts {
    use gz_project::asset_kind::MediaKind;
    if let Some(e) = index.by_name(MediaKind::Sound, name) {
        let read = crate::resolve::sound(root, e.path.as_str()).ok().map(|(a, _, _)| a);
        let copyrighted = read.as_ref().is_some_and(SoundAsset::is_copyrighted);
        return Facts {
            mark: if copyrighted { Mark::Copyrighted } else { Mark::Free },
            instead: read.and_then(|a| a.streamer).filter(|_| copyrighted).unwrap_or_default(),
            asset: Some(e.path.as_str().to_owned()),
        };
    }
    let free = Facts { mark: Mark::Free, instead: String::new(), asset: None };
    if !game.linked {
        return free;
    }
    if !game.has_sound(name) {
        return Facts { mark: Mark::Missing, ..free };
    }
    match right_of(&game.rights, name) {
        Some(r) => Facts { mark: Mark::Copyrighted, instead: r.instead.clone(), asset: None },
        None => free,
    }
}

/// A sound that plays in the streamer mode, as a picker offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeSound {
    /// The game name.
    pub name: String,
    /// The module's sound asset that publishes it; `None` for a sound of the game.
    pub asset: Option<String>,
}

/// Every sound that plays in the streamer mode: the module's sound assets without the mark
/// (sorted), then the linked game's sounds its own list does not name (sorted), each name once.
/// Blocking.
#[must_use]
pub fn free_sounds(
    root: &std::path::Path,
    index: &crate::index::Index,
    game: &crate::service::GameLists,
) -> Vec<FreeSound> {
    use gz_project::asset_kind::MediaKind;
    let mut out: Vec<FreeSound> = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();
    let mut own: Vec<&crate::index::Entry> =
        index.of_kind(MediaKind::Sound).filter(|e| !e.excluded && e.error.is_none()).collect();
    own.sort_by_key(|e| e.name.to_lowercase());
    for e in own {
        let free = crate::resolve::sound(root, e.path.as_str()).is_ok_and(|(a, _, _)| !a.is_copyrighted());
        if free && seen.insert(e.key.clone()) {
            out.push(FreeSound { name: e.name.clone(), asset: Some(e.path.as_str().to_owned()) });
        }
    }
    if let Some(files) = &game.files {
        let mut names: Vec<String> = files
            .find("sounds", "ogg")
            .into_iter()
            .filter_map(|f| {
                let rel = f.path.strip_prefix("sounds/")?;
                Some(rel.rsplit_once('.').map_or(rel, |(s, _)| s).replace('/', "\\"))
            })
            .collect();
        names.sort();
        for n in names {
            let key = read::name_key(&n);
            if !listed(&game.rights, &key) && seen.insert(key) {
                out.push(FreeSound { name: n, asset: None });
            }
        }
    }
    out
}
