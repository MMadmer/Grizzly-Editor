//! An item as plain data: read from its file with every problem of its own shape at the key it
//! is about ([`from_value`]), projected back to a value for a save to compare ([`to_value`]: the
//! save writes only the difference into the author's text), written canonically for a new file
//! ([`write`], [`template`]).
//!
//! **Unset stays unset**: a key the file leaves out is `None`, an empty list or `Value::Nil`,
//! never a default - what the base gives is the base's, and a change changes only what it says.

use gz_base::Severity;
use gz_luadata::{DiffRules, Seg, Table, Value, number_to_text, write_asset};

use crate::fields;

/// The header key of an item file and the format version this editor reads up to.
pub const HEADER: &str = "item";
pub const VERSION: u32 = 1;

/// A value outside what its key takes.
pub const BAD_VALUE: &str = "I427";

/// A problem of a file found while it is read, at the key it is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: &'static str,
    pub severity: Severity,
    /// Where in the file: a path of keys and list items; empty for the whole file.
    pub path: Vec<Seg>,
    pub message: String,
}

impl Issue {
    #[must_use]
    pub fn new(code: &'static str, severity: Severity, path: Vec<Seg>, message: impl Into<String>) -> Self {
        Self { code, severity, path, message: message.into() }
    }

    #[must_use]
    pub fn error(code: &'static str, path: Vec<Seg>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, path, message)
    }

    #[must_use]
    pub fn warning(code: &'static str, path: Vec<Seg>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, path, message)
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// A value of a key of a section, as the asset writes it.
#[derive(Clone, Debug, PartialEq)]
pub enum SetValue {
    Text(String),
    Number(f64),
    Bool(bool),
    /// `{ a, b }`: `a, b` in the game.
    List(Vec<SetValue>),
}

impl SetValue {
    /// A string, a number, a boolean, or a flat list of them.
    #[must_use]
    pub fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::String(s) => Some(Self::Text(s.to_string())),
            Value::Number(n) => Some(Self::Number(*n)),
            Value::Bool(b) => Some(Self::Bool(*b)),
            Value::Table(t) if t.fields.is_empty() => t
                .arr
                .iter()
                .map(|x| match x {
                    Value::Table(_) | Value::Nil => None,
                    other => Self::from_value(other),
                })
                .collect::<Option<Vec<_>>>()
                .map(Self::List),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(s) => Value::from(s.as_str()),
            Self::Number(n) => Value::Number(*n),
            Self::Bool(b) => Value::Bool(*b),
            Self::List(l) => Value::from(l.iter().map(Self::to_value).collect::<Vec<_>>()),
        }
    }

    /// The value as the game reads it from a config line.
    #[must_use]
    pub fn ltx(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Number(n) => number_to_text(*n),
            Self::Bool(b) => b.to_string(),
            Self::List(l) => l.iter().map(Self::ltx).collect::<Vec<_>>().join(", "),
        }
    }
}

/// `set`, `unset`, `add`, `drop` of a section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub set: Vec<(String, SetValue)>,
    pub unset: Vec<String>,
    pub add: Vec<(String, Vec<String>)>,
    pub drop: Vec<(String, Vec<String>)>,
}

impl Changes {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.unset.is_empty() && self.add.is_empty() && self.drop.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&SetValue> {
        self.set.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// The icon in one of its forms.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// A rectangle of a texture in cells of 50 px; no texture: the game's atlas (the game takes
    /// `icons_texture` away); `w` and `h` left out keep the section's size.
    Cell { texture: Option<String>, x: u32, y: u32, w: Option<u32>, h: Option<u32> },
    /// A picture beside the asset that the module build bakes into the module's atlas; `w` and
    /// `h` left out are its size in cells. `plain`: written as the file name alone.
    Picture { file: String, w: Option<u32>, h: Option<u32>, plain: bool },
}

/// The item's HUD section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub base: Vec<String>,
    pub changes: Changes,
}

/// A row of `sections`: one the item makes (`name`) or one it changes (`edit`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnedSection {
    pub name: String,
    pub edit: String,
    pub base: Vec<String>,
    pub changes: Changes,
}

/// A recipe that makes the item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Craft {
    pub at: String,
    pub count: Option<u32>,
    pub parts: Vec<String>,
}

/// How many the dead carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Count {
    One(f64),
    Range(f64, f64),
}

/// Death loot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loot {
    pub count: Option<Count>,
    pub chance: Option<f64>,
    pub by: Vec<(String, f64)>,
}

/// A row of a trader's stock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sold {
    pub by: String,
    pub supplies: Vec<String>,
    pub count: Option<f64>,
    pub chance: Option<f64>,
}

impl Sold {
    /// The trader's config under `configs`: `misc\trade\<by>.ltx`, or the path a `by` ending in
    /// `.ltx` is; `None` when it names no config under `configs`.
    #[must_use]
    pub fn file(&self) -> Option<String> {
        let name = self.by.to_ascii_lowercase();
        let path =
            if name.len() > 4 && name.ends_with(".ltx") { name } else { format!("misc\\trade\\{name}.ltx") };
        gz_ltx::patch::config_path(&path).filter(|p| !p.is_empty() && !p.contains(':') && plain_value(p))
    }

    /// The supplies sections, lower case; `supplies_1` when the row names none.
    #[must_use]
    pub fn sections(&self) -> Vec<String> {
        if self.supplies.is_empty() {
            vec!["supplies_1".to_owned()]
        } else {
            self.supplies.iter().map(|s| s.to_ascii_lowercase()).collect()
        }
    }
}

/// The use scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Scene {
    /// The scene of that item.
    Like(String),
    /// The keys of `animations_settings.ltx`, over `like` when given, and its own HUD section.
    Table { like: Option<String>, keys: Vec<(String, SetValue)>, hud: Option<Hud> },
}

/// A row of `lists`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListRow {
    pub file: Option<String>,
    pub section: Option<String>,
    pub key: Option<String>,
    pub value: Option<SetValue>,
    pub add: Option<String>,
    pub drop: Option<String>,
}

/// What a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A new item: `base`, `id`, `section` or `abstract`.
    New,
    /// A change of one that exists: `edit`.
    Change,
    /// Neither: nothing says what the file is.
    Neither,
    /// Both at once.
    Both,
}

/// One item of the module.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// Where it was read from: project-relative with `/`.
    pub file: String,
    pub id: Option<String>,
    pub section: Option<String>,
    pub base: Vec<String>,
    pub is_abstract: Option<bool>,
    pub edit: Option<String>,
    /// A string (Russian) or a table by language; `Nil`: unset.
    pub name: Value,
    pub short_name: Value,
    pub description: Value,
    pub icon: Option<Icon>,
    pub changes: Changes,
    pub hud: Option<Hud>,
    pub sections: Vec<OwnedSection>,
    pub craft: Vec<Craft>,
    pub loot: Option<Loot>,
    pub sold: Vec<Sold>,
    pub scene: Option<Scene>,
    pub lists: Vec<ListRow>,
    /// Kept as it is: the editor's notes.
    pub editor: Value,
}

/// The base name of a path without its extension.
#[must_use]
pub fn stem(path: &str) -> &str {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match leaf.rfind('.') {
        Some(dot) if dot > 0 => leaf.get(..dot).unwrap_or(leaf),
        _ => leaf,
    }
}

/// `a-z 0-9 _ . -`, not empty, under 64 characters: what a save reads into its 64 bytes.
#[must_use]
pub fn valid_section(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-'))
}

impl Item {
    #[must_use]
    pub fn kind(&self) -> Kind {
        let new = self.id.is_some()
            || self.section.is_some()
            || !self.base.is_empty()
            || self.is_abstract.is_some();
        match (new, self.edit.is_some()) {
            (true, true) => Kind::Both,
            (true, false) => Kind::New,
            (false, true) => Kind::Change,
            (false, false) => Kind::Neither,
        }
    }

    #[must_use]
    pub fn is_change(&self) -> bool {
        self.kind() == Kind::Change
    }

    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.is_abstract == Some(true)
    }

    /// A new item's id: its `id`, else its file's name.
    #[must_use]
    pub fn id_or_stem(&self) -> String {
        match &self.id {
            Some(id) => id.clone(),
            None => gz_characters::model::id_from_name(stem(&self.file)),
        }
    }

    /// The section the item makes or changes, lower case: `edit`, `section`, else
    /// `<module>_<id>`.
    #[must_use]
    pub fn game_section(&self, module: &str) -> String {
        if let Some(e) = &self.edit {
            return e.to_ascii_lowercase();
        }
        if let Some(s) = &self.section {
            return s.to_ascii_lowercase();
        }
        format!("{module}_{}", self.id_or_stem())
    }

    /// The HUD section a new item with `hud` makes: always `<section>_hud` (`set.hud` may still
    /// point the item's `hud` elsewhere).
    #[must_use]
    pub fn hud_section(&self, module: &str) -> Option<String> {
        self.hud.as_ref()?;
        (self.kind() == Kind::New).then(|| format!("{}_hud", self.game_section(module)))
    }

    /// The picture of an icon of the authoring form, as written.
    #[must_use]
    pub fn picture(&self) -> Option<&str> {
        match &self.icon {
            Some(Icon::Picture { file, .. }) => Some(file.as_str()),
            _ => None,
        }
    }
}

// ---- reading ---------------------------------------------------------------------------------

/// A section or a key a config line can hold (the game's `plain_name`): 1 to 255 characters,
/// no blank, control character or `[ ] = ; , "`.
#[must_use]
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.bytes().all(|c| c >= 0x21 && c != 0x7f && !b"[]=;,\"".contains(&c))
}

/// A value a config line can hold (the game's `plain_value`): no line break or other control
/// character but a tab.
#[must_use]
pub fn plain_value(value: &str) -> bool {
    value.bytes().all(|c| (c >= 0x20 || c == b'\t') && c != 0x7f)
}

/// What a read gathers: where it is in the file and the issues so far.
#[derive(Default)]
struct Cx {
    path: Vec<Seg>,
    issues: Vec<Issue>,
}

impl Cx {
    fn at(&self, key: &str) -> Vec<Seg> {
        let mut p = self.path.clone();
        p.push(Seg::key(key));
        p
    }

    fn bad(&mut self, key: &str, message: impl Into<String>) {
        let path = self.at(key);
        self.issues.push(Issue::error(BAD_VALUE, path, message));
    }

    fn bad_at(&mut self, path: Vec<Seg>, message: impl Into<String>) {
        self.issues.push(Issue::error(BAD_VALUE, path, message));
    }

    fn within<R>(&mut self, segs: &[Seg], f: impl FnOnce(&mut Self) -> R) -> R {
        let depth = self.path.len();
        self.path.extend_from_slice(segs);
        let r = f(self);
        self.path.truncate(depth);
        r
    }

    fn text(&mut self, v: &Value, key: &str) -> Option<String> {
        let raw = v.get(key)?;
        match raw.as_str() {
            Some(s) => Some(s.to_owned()),
            None => {
                self.bad(key, format!("'{key}' takes a string"));
                None
            }
        }
    }

    /// A section a key names: a string a config can hold as a name.
    fn name(&mut self, v: &Value, key: &str, what: &str) -> Option<String> {
        let s = self.text(v, key)?;
        if !plain_name(&s.to_ascii_lowercase()) {
            self.bad(key, format!("{what} names a section no config can hold"));
            return None;
        }
        Some(s)
    }

    fn boolean(&mut self, v: &Value, key: &str) -> Option<bool> {
        let raw = v.get(key)?;
        let b = raw.as_bool();
        if b.is_none() {
            self.bad(key, format!("'{key}' takes true or false"));
        }
        b
    }

    /// Any number: what the game takes.
    fn any_number(&mut self, v: &Value, key: &str) -> Option<f64> {
        let raw = v.get(key)?;
        match raw.as_number().filter(|n| n.is_finite()) {
            Some(n) => Some(n),
            None => {
                self.bad(key, format!("'{key}' takes a number"));
                None
            }
        }
    }

    fn integer(&mut self, v: &Value, fields: &[fields::Field], key: &str) -> Option<u32> {
        let n = self.any_number(v, key)?;
        let (min, max) = match fields::field(fields, key).map(|f| f.ty) {
            Some(fields::Ty::Integer { min, max }) => (min as f64, max as f64),
            _ => (0.0, f64::from(u32::MAX)),
        };
        if n.fract() != 0.0 || n < min || n > max {
            self.bad(key, format!("'{key}' is {n}; it takes a whole number from {min} to {max}"));
            return None;
        }
        Some(n as u32)
    }

    /// A string or a list of strings.
    fn names(&mut self, v: &Value, key: &str, what: &str) -> Vec<String> {
        match v.get(key) {
            None => Vec::new(),
            Some(Value::String(s)) => vec![s.to_string()],
            Some(raw)
                if raw.is_table() && raw.fields().is_empty() && raw.arr().iter().all(Value::is_string) =>
            {
                raw.arr().iter().filter_map(Value::as_str).map(str::to_owned).collect()
            }
            Some(_) => {
                self.bad(key, format!("{what} is a string or a list of them"));
                Vec::new()
            }
        }
    }

    /// Sections a key names: one or a list, each one a config can hold.
    fn sections(&mut self, v: &Value, key: &str, what: &str) -> Vec<String> {
        let list = self.names(v, key, what);
        if list.iter().any(|s| !plain_name(&s.to_ascii_lowercase())) {
            self.bad(key, format!("{what} names a section no config can hold"));
            return Vec::new();
        }
        list
    }

    /// The rows of a list key; a key that is no list is an issue and reads as empty.
    fn rows<'v>(&mut self, v: &'v Value, key: &str, what: &str) -> &'v [Value] {
        match v.get(key) {
            None => &[],
            Some(raw) if raw.is_table() && raw.fields().is_empty() => raw.arr(),
            Some(_) => {
                self.bad(key, what.to_owned());
                &[]
            }
        }
    }

    fn text_value(&mut self, v: &Value, key: &str) -> Value {
        match v.get(key) {
            None => Value::Nil,
            Some(s @ Value::String(_)) => s.clone(),
            Some(t @ Value::Table(tt)) if tt.fields.iter().any(|(_, x)| x.is_string()) => t.clone(),
            Some(_) => {
                self.bad(key, format!("{key} is a string or {{ language = text }}"));
                Value::Nil
            }
        }
    }

    fn keys(&mut self, v: &Value, key: &str) -> Vec<(String, SetValue)> {
        let Some(raw) = v.get(key) else { return Vec::new() };
        let Some(t) = raw.as_table().filter(|t| t.arr.is_empty()) else {
            self.bad(key, format!("{key} takes key = value"));
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, x) in &t.fields {
            let mut p = self.at(key);
            p.push(Seg::Key(k.clone()));
            if !plain_name(k) {
                self.bad_at(p, format!("'{k}' cannot be a key of a config"));
                continue;
            }
            match SetValue::from_value(x) {
                Some(sv) if plain_value(&sv.ltx()) => out.push((k.to_string(), sv)),
                Some(_) => self.bad_at(p, format!("the value of {k} holds characters no config can carry")),
                None => {
                    self.bad_at(p, format!("the value of {k} is no string, number, boolean or list of them"))
                }
            }
        }
        out
    }

    fn key_names(&mut self, v: &Value, key: &str) -> Vec<String> {
        let list = self.names(v, key, &format!("{key}, a list of keys,"));
        for k in &list {
            if !plain_name(k) {
                self.bad(key, format!("'{k}' cannot be a key of a config"));
                return Vec::new();
            }
        }
        list
    }

    /// `{ key = member or list }`: a member is a string, a number or a boolean.
    fn members(&mut self, v: &Value, key: &str) -> Vec<(String, Vec<String>)> {
        let Some(raw) = v.get(key) else { return Vec::new() };
        let Some(t) = raw.as_table().filter(|t| t.arr.is_empty()) else {
            self.bad(key, format!("{key} takes key = member or {{ members }}"));
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, x) in &t.fields {
            let mut p = self.at(key);
            p.push(Seg::Key(k.clone()));
            if !plain_name(k) {
                self.bad_at(p, format!("'{k}' cannot be a key of a config"));
                continue;
            }
            let scalar = |x: &Value| match x {
                Value::Table(_) | Value::Nil => None,
                other => SetValue::from_value(other).map(|s| s.ltx()),
            };
            let list: Option<Vec<String>> = match x {
                Value::Table(tt) if tt.fields.is_empty() => tt.arr.iter().map(scalar).collect(),
                Value::Table(_) => None,
                other => scalar(other).filter(|s| !s.is_empty()).map(|s| vec![s]),
            };
            match list {
                None => self.bad_at(p, format!("{key}.{k} is a member or a list of them")),
                Some(l) if l.iter().any(|m| !plain_value(m) || m.contains(',')) => {
                    self.bad_at(p, format!("{key}.{k} holds a member no list can carry"));
                }
                Some(l) => out.push((k.to_string(), l)),
            }
        }
        out
    }

    fn changes(&mut self, v: &Value) -> Changes {
        Changes {
            set: self.keys(v, "set"),
            unset: self.key_names(v, "unset"),
            add: self.members(v, "add"),
            drop: self.members(v, "drop"),
        }
    }

    fn hud(&mut self, v: &Value) -> Hud {
        Hud { base: self.sections(v, "base", "hud.base"), changes: self.changes(v) }
    }

    fn icon(&mut self, v: &Value) -> Option<Icon> {
        let raw = v.get("icon")?;
        if let Some(s) = raw.as_str() {
            return Some(Icon::Picture { file: s.to_owned(), w: None, h: None, plain: true });
        }
        if !raw.is_table() {
            self.bad("icon", "icon is { texture, x, y, w, h }, in cells, or a picture beside the asset");
            return None;
        }
        self.within(&[Seg::key("icon")], |cx| {
            let w = cx.integer(raw, fields::ICON, "w");
            let h = cx.integer(raw, fields::ICON, "h");
            if raw.get("file").is_some() {
                let file = cx.text(raw, "file")?;
                return Some(Icon::Picture { file, w, h, plain: false });
            }
            let texture = cx.text(raw, "texture");
            if texture.as_deref().is_some_and(|t| t.is_empty() || !plain_value(t)) {
                cx.bad("texture", "icon.texture names a texture");
            }
            let (x, y) = (cx.integer(raw, fields::ICON, "x"), cx.integer(raw, fields::ICON, "y"));
            if raw.get("x").is_none() || raw.get("y").is_none() {
                cx.issues.push(Issue::error(
                    BAD_VALUE,
                    cx.path.clone(),
                    "icon is { texture, x, y, w, h }, in cells: x and y are its place",
                ));
            }
            Some(Icon::Cell { texture, x: x.unwrap_or(0), y: y.unwrap_or(0), w, h })
        })
    }

    fn loot(&mut self, v: &Value) -> Option<Loot> {
        let raw = v.get("loot")?;
        if !raw.as_table().is_some_and(|t| t.arr.is_empty()) {
            self.bad("loot", "loot is { count, chance, by }");
            return None;
        }
        Some(self.within(&[Seg::key("loot")], |cx| {
            let count = match raw.get("count") {
                None => None,
                Some(Value::Number(n)) => Some(Count::One(*n)),
                Some(Value::Table(t))
                    if t.fields.is_empty()
                        && t.arr.len() == 2
                        && t.arr.iter().all(|x| x.as_number().is_some()) =>
                {
                    let n = |i: usize| t.arr.get(i).and_then(Value::as_number).unwrap_or(0.0);
                    Some(Count::Range(n(0), n(1)))
                }
                Some(_) => {
                    cx.bad("count", "loot.count is a number or { min, max }");
                    None
                }
            };
            let chance = cx.any_number(raw, "chance");
            let mut by = Vec::new();
            match raw.get("by") {
                None => {}
                Some(Value::Table(t)) if t.arr.is_empty() => {
                    for (k, x) in &t.fields {
                        match x.as_number().filter(|_| plain_name(&k.to_ascii_lowercase())) {
                            Some(n) => by.push((k.to_string(), n)),
                            None => cx.bad_at(
                                vec![Seg::key("loot"), Seg::key("by"), Seg::Key(k.clone())],
                                "loot.by is { <community or community_rank> = chance }",
                            ),
                        }
                    }
                }
                Some(_) => cx.bad("by", "loot.by is { <community or community_rank> = chance }"),
            }
            Loot { count, chance, by }
        }))
    }

    fn sold_row(&mut self, row: &Value) -> Option<Sold> {
        let by = match row.get("by").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            Some(s) => s.to_owned(),
            None => {
                self.issues.push(Issue::error(
                    BAD_VALUE,
                    self.at("by"),
                    "a row of sold names the trader's config: by",
                ));
                return None;
            }
        };
        let s = Sold {
            supplies: self.sections(row, "supplies", "sold.supplies"),
            count: self.any_number(row, "count"),
            chance: self.any_number(row, "chance"),
            by,
        };
        if s.file().is_none() {
            self.bad("by", "sold.by is a trader's config, or a config's path under configs");
        }
        Some(s)
    }

    fn scene(&mut self, v: &Value) -> Option<Scene> {
        let raw = v.get("scene")?;
        if let Some(s) = raw.as_str() {
            if !plain_name(&s.to_ascii_lowercase()) {
                self.bad("scene", "scene.like names the item whose scene it takes");
            }
            return Some(Scene::Like(s.to_owned()));
        }
        if !raw.is_table() {
            self.bad("scene", "scene is the item whose scene it takes, or { like, sound, camera, time, helmet, anm, hud_anm, hud }");
            return None;
        }
        Some(self.within(&[Seg::key("scene")], |cx| {
            let like = cx.name(raw, "like", "scene.like");
            let mut keys = Vec::new();
            for f in fields::SCENE.iter().filter(|f| !matches!(f.key, "like" | "hud")) {
                if let Some(x) = raw.get(f.key) {
                    match SetValue::from_value(x).filter(|s| plain_value(&s.ltx())) {
                        Some(sv) => keys.push((f.key.to_owned(), sv)),
                        None => cx.bad(f.key, format!("scene.{} is a value no config can carry", f.key)),
                    }
                }
            }
            let hud = match raw.get("hud") {
                None => None,
                Some(h) if h.is_table() => Some(cx.within(&[Seg::key("hud")], |cx| cx.hud(h))),
                Some(_) => {
                    cx.bad("hud", "scene.hud is { base, set, unset, add, drop }");
                    None
                }
            };
            Scene::Table { like, keys, hud }
        }))
    }
}

/// An item of a file's value, with the problems of its own shape: any of them makes the game
/// leave the whole item out. A file of a newer format version is refused.
pub fn from_value(v: &Value) -> Result<(Item, Vec<Issue>), String> {
    if !v.is_table() {
        return Err("an item file returns one table".into());
    }
    if let Some(Value::Number(n)) = v.get(HEADER)
        && *n > f64::from(VERSION)
    {
        return Err(format!(
            "made by a newer Grizzly Editor ({HEADER} = {n}; this one reads up to {VERSION}): update the editor to open it"
        ));
    }
    let mut cx = Cx::default();
    let mut item = Item {
        id: cx.text(v, "id"),
        section: cx.text(v, "section"),
        base: cx.sections(v, "base", "base"),
        is_abstract: cx.boolean(v, "abstract"),
        edit: cx.name(v, "edit", "edit"),
        name: cx.text_value(v, "name"),
        short_name: cx.text_value(v, "short_name"),
        description: cx.text_value(v, "description"),
        changes: cx.changes(v),
        editor: v.get("editor").cloned().unwrap_or_default(),
        ..Item::default()
    };
    item.icon = cx.icon(v);
    if let Some(h) = v.get("hud") {
        if h.is_table() {
            item.hud = Some(cx.within(&[Seg::key("hud")], |cx| cx.hud(h)));
            if item.edit.is_some() && h.get("base").is_some() {
                cx.bad_at(
                    vec![Seg::key("hud"), Seg::key("base")],
                    "hud of a change changes the section the item's hud names: base belongs to a new item's hud",
                );
            }
        } else {
            cx.bad("hud", "hud is { base, set, unset, add, drop }");
        }
    }
    let rows = cx.rows(v, "sections", "sections is a list of { name, base, set } and { edit, set }").to_vec();
    for (i, row) in rows.iter().enumerate() {
        let r = cx.within(&[Seg::key("sections"), Seg::Index(i)], |cx| {
            let named = |k: &str| row.get(k).is_some();
            if !row.is_table() || named("name") == named("edit") {
                cx.issues.push(Issue::error(
                    BAD_VALUE,
                    cx.path.clone(),
                    "a row of sections has a name (a section it makes) or edit (one it changes)",
                ));
                return None;
            }
            let s = OwnedSection {
                name: cx.text(row, "name").unwrap_or_default(),
                edit: cx.name(row, "edit", "edit").unwrap_or_default(),
                base: cx.sections(row, "base", "the base"),
                changes: cx.changes(row),
            };
            if !s.edit.is_empty() && named("base") {
                cx.bad(
                    "base",
                    format!(
                        "[{}] exists and keeps its parents: base belongs to a section a row makes",
                        s.edit.to_ascii_lowercase()
                    ),
                );
            }
            Some(s)
        });
        item.sections.extend(r);
    }
    let rows = cx.rows(v, "craft", "craft is a list of { at, count, parts }").to_vec();
    for (i, row) in rows.iter().enumerate() {
        let r = cx.within(&[Seg::key("craft"), Seg::Index(i)], |cx| {
            if !row.is_table() {
                cx.issues.push(Issue::error(BAD_VALUE, cx.path.clone(), "a recipe is { at, count, parts }"));
                return None;
            }
            let at = cx.name(row, "at", "a recipe's at");
            if at.is_none() && row.get("at").is_none() {
                cx.bad("at", "a recipe names the tool or workbench it is made at: at");
            }
            let parts = cx.sections(row, "parts", "a part of a recipe");
            if parts.is_empty() || parts.len() > 4 {
                cx.bad("parts", "a recipe takes one to four parts");
            }
            Some(Craft { at: at.unwrap_or_default(), count: cx.integer(row, fields::CRAFT, "count"), parts })
        });
        item.craft.extend(r);
    }
    item.loot = cx.loot(v);
    match v.get("sold") {
        None => {}
        // one row may stand alone
        Some(row @ Value::Table(t)) if !t.fields.is_empty() => {
            let r = cx.within(&[Seg::key("sold")], |cx| cx.sold_row(row));
            item.sold.extend(r);
        }
        Some(Value::Table(t)) => {
            for (i, row) in t.arr.iter().enumerate() {
                let r = cx.within(&[Seg::key("sold"), Seg::Index(i)], |cx| {
                    if !row.is_table() {
                        cx.issues.push(Issue::error(
                            BAD_VALUE,
                            cx.path.clone(),
                            "a row of sold is { by, supplies, count, chance }",
                        ));
                        return None;
                    }
                    cx.sold_row(row)
                });
                item.sold.extend(r);
            }
        }
        Some(_) => cx.bad("sold", "sold is a list of { by, supplies, count, chance }"),
    }
    item.scene = cx.scene(v);
    let rows = cx
        .rows(
            v,
            "lists",
            "lists is a list of { file, section, key, value } and { file, section, key, add | drop }",
        )
        .to_vec();
    for (i, row) in rows.iter().enumerate() {
        let r = cx.within(&[Seg::key("lists"), Seg::Index(i)], |cx| {
            if !row.is_table() {
                cx.issues.push(Issue::error(
                    BAD_VALUE,
                    cx.path.clone(),
                    "a row of lists is { file, section, key, value }",
                ));
                return None;
            }
            let value = match row.get("value") {
                None => None,
                Some(x) => match SetValue::from_value(x).filter(|s| plain_value(&s.ltx())) {
                    Some(sv) => Some(sv),
                    None => {
                        cx.bad("value", "a row of lists holds a value no config can carry");
                        None
                    }
                },
            };
            let l = ListRow {
                file: cx.text(row, "file"),
                section: cx.name(row, "section", "a row of lists"),
                key: cx.text(row, "key"),
                value,
                add: cx.text(row, "add"),
                drop: cx.text(row, "drop"),
            };
            if l.file.as_deref().is_some_and(|f| {
                gz_ltx::patch::config_path(f).is_none_or(|p| p.is_empty() || p.contains(':'))
            }) {
                cx.bad("file", "a row of lists names a config by its path under configs");
            }
            if l.section.is_none() && row.get("section").is_none() {
                cx.bad("section", "a row of lists names its section");
            }
            if l.key.as_deref().is_some_and(|k| !plain_name(k)) {
                cx.bad("key", "a row of lists names a key a config can hold");
            }
            let ways =
                [row.get("value"), row.get("add"), row.get("drop")].iter().filter(|x| x.is_some()).count();
            if ways > 1 {
                cx.issues.push(Issue::error(
                    BAD_VALUE,
                    cx.path.clone(),
                    "a row of lists sets a value, adds to a list or drops from one: one of them",
                ));
            }
            for m in [&l.add, &l.drop].into_iter().flatten() {
                if !plain_value(m) || m.contains(',') {
                    cx.issues.push(Issue::error(
                        BAD_VALUE,
                        cx.path.clone(),
                        "a row of lists holds a member no list can carry",
                    ));
                }
            }
            Some(l)
        });
        item.lists.extend(r);
    }
    Ok((item, cx.issues))
}

// Reads the text of an item file. `file` is what [`Item::file`] says; the error is a
/// sentence, `line N: <message>` when the reader names a line.
pub fn load_text(text: &str, file: &str) -> Result<(Item, Vec<Issue>), String> {
    let v = gz_luadata::parse_named(text, stem(file))
        .map_err(|e| if e.line != 0 { format!("line {}: {}", e.line, e.message) } else { e.message })?;
    let (mut item, issues) = from_value(&v)?;
    file.clone_into(&mut item.file);
    Ok((item, issues))
}

/// Reads an item file: at most 16 MB, a UTF-8 byte order mark left out.
pub fn read_file(path: &std::path::Path) -> Result<String, String> {
    gz_characters::model::read_file(path)
}

// ---- writing ---------------------------------------------------------------------------------

fn names_value(list: &[String]) -> Value {
    match list {
        [one] => Value::from(one.as_str()),
        many => Value::from(many.iter().map(|n| Value::from(n.as_str())).collect::<Vec<_>>()),
    }
}

fn put_changes(t: &mut Table, c: &Changes) {
    if !c.set.is_empty() {
        let mut s = Table::new();
        for (k, v) in &c.set {
            s.put(k, v.to_value());
        }
        t.put("set", s);
    }
    if !c.unset.is_empty() {
        t.put("unset", Value::from(c.unset.iter().map(|k| Value::from(k.as_str())).collect::<Vec<_>>()));
    }
    for (key, list) in [("add", &c.add), ("drop", &c.drop)] {
        if !list.is_empty() {
            let mut m = Table::new();
            for (k, members) in list {
                m.put(k, names_value(members));
            }
            t.put(key, m);
        }
    }
}

fn hud_value(h: &Hud) -> Value {
    let mut t = Table::new();
    if !h.base.is_empty() {
        t.put("base", names_value(&h.base));
    }
    put_changes(&mut t, &h.changes);
    Value::Table(t)
}

fn text_value(v: &Value) -> Option<Value> {
    let mut v = v.clone();
    gz_characters::text_collapse(&mut v);
    (!gz_characters::text_empty(&v)).then_some(v)
}

/// An item as a save compares it with what was read, and as a new file holds it: the keys in
/// the reference's order, nothing the file did not say.
#[must_use]
pub fn to_value(item: &Item) -> Value {
    let mut t = Table::new();
    t.put(HEADER, VERSION);
    if let Some(id) = &item.id {
        t.put("id", id.as_str());
    }
    if let Some(s) = &item.section {
        t.put("section", s.as_str());
    }
    if !item.base.is_empty() {
        t.put("base", names_value(&item.base));
    }
    if let Some(a) = item.is_abstract {
        t.put("abstract", a);
    }
    if let Some(e) = &item.edit {
        t.put("edit", e.as_str());
    }
    for (key, v) in
        [("name", &item.name), ("short_name", &item.short_name), ("description", &item.description)]
    {
        if let Some(v) = text_value(v) {
            t.put(key, v);
        }
    }
    match &item.icon {
        None => {}
        Some(Icon::Picture { file, w: None, h: None, plain: true }) => t.put("icon", file.as_str()),
        Some(Icon::Picture { file, w, h, .. }) => {
            let mut i = Table::new();
            i.put("file", file.as_str());
            if let Some(w) = w {
                i.put("w", *w);
            }
            if let Some(h) = h {
                i.put("h", *h);
            }
            t.put("icon", i);
        }
        Some(Icon::Cell { texture, x, y, w, h }) => {
            let mut i = Table::new();
            if let Some(tex) = texture {
                i.put("texture", tex.as_str());
            }
            i.put("x", *x);
            i.put("y", *y);
            if let Some(w) = w {
                i.put("w", *w);
            }
            if let Some(h) = h {
                i.put("h", *h);
            }
            t.put("icon", i);
        }
    }
    put_changes(&mut t, &item.changes);
    if let Some(h) = &item.hud {
        t.put("hud", hud_value(h));
    }
    if !item.sections.is_empty() {
        let rows: Vec<Value> = item
            .sections
            .iter()
            .map(|s| {
                let mut r = Table::new();
                if !s.name.is_empty() {
                    r.put("name", s.name.as_str());
                }
                if !s.edit.is_empty() {
                    r.put("edit", s.edit.as_str());
                }
                if !s.base.is_empty() {
                    r.put("base", names_value(&s.base));
                }
                put_changes(&mut r, &s.changes);
                Value::Table(r)
            })
            .collect();
        t.put("sections", Value::from(rows));
    }
    if !item.craft.is_empty() {
        let rows: Vec<Value> = item
            .craft
            .iter()
            .map(|c| {
                let mut r = Table::new();
                r.put("at", c.at.as_str());
                if let Some(n) = c.count {
                    r.put("count", n);
                }
                r.put(
                    "parts",
                    Value::from(c.parts.iter().map(|p| Value::from(p.as_str())).collect::<Vec<_>>()),
                );
                Value::Table(r)
            })
            .collect();
        t.put("craft", Value::from(rows));
    }
    if let Some(l) = &item.loot {
        let mut r = Table::new();
        match l.count {
            None => {}
            Some(Count::One(n)) => r.put("count", n),
            Some(Count::Range(a, b)) => r.put("count", Value::from(vec![Value::from(a), Value::from(b)])),
        }
        if let Some(c) = l.chance {
            r.put("chance", c);
        }
        if !l.by.is_empty() {
            let mut by = Table::new();
            for (k, c) in &l.by {
                by.put(k, *c);
            }
            r.put("by", by);
        }
        t.put("loot", r);
    }
    if !item.sold.is_empty() {
        let rows: Vec<Value> = item
            .sold
            .iter()
            .map(|s| {
                let mut r = Table::new();
                r.put("by", s.by.as_str());
                if !s.supplies.is_empty() {
                    r.put("supplies", names_value(&s.supplies));
                }
                if let Some(n) = s.count {
                    r.put("count", n);
                }
                if let Some(c) = s.chance {
                    r.put("chance", c);
                }
                Value::Table(r)
            })
            .collect();
        t.put("sold", Value::from(rows));
    }
    match &item.scene {
        None => {}
        Some(Scene::Like(s)) => t.put("scene", s.as_str()),
        Some(Scene::Table { like, keys, hud }) => {
            let mut r = Table::new();
            if let Some(l) = like {
                r.put("like", l.as_str());
            }
            for (k, v) in keys {
                r.put(k, v.to_value());
            }
            if let Some(h) = hud {
                r.put("hud", hud_value(h));
            }
            t.put("scene", r);
        }
    }
    if !item.lists.is_empty() {
        let rows: Vec<Value> = item
            .lists
            .iter()
            .map(|l| {
                let mut r = Table::new();
                for (k, v) in [("file", &l.file), ("section", &l.section), ("key", &l.key)] {
                    if let Some(v) = v {
                        r.put(k, v.as_str());
                    }
                }
                if let Some(v) = &l.value {
                    r.put("value", v.to_value());
                }
                for (k, v) in [("add", &l.add), ("drop", &l.drop)] {
                    if let Some(v) = v {
                        r.put(k, v.as_str());
                    }
                }
                Value::Table(r)
            })
            .collect();
        t.put("lists", Value::from(rows));
    }
    if !item.editor.is_nil() {
        t.put("editor", item.editor.clone());
    }
    Value::Table(t)
}

/// `{ texture, x, y, w, h }`: the form the game reads.
#[must_use]
pub fn cell_value(texture: Option<&str>, x: u32, y: u32, w: u32, h: u32) -> Value {
    let mut i = Table::new();
    if let Some(tex) = texture {
        i.put("texture", tex);
    }
    i.put("x", x);
    i.put("y", y);
    i.put("w", w);
    i.put("h", h);
    Value::Table(i)
}

/// The canonical text of a new item file. An existing file is never written this way: a save
/// writes only what changed into it ([`to_value`]).
#[must_use]
pub fn write(item: &Item) -> String {
    let head = if item.is_change() {
        "-- A change of an item that exists. `Grizzly-Cmd.exe reference item` lists every key.\n"
    } else {
        "-- An item of this module. `Grizzly-Cmd.exe reference item` lists every key.\n"
    };
    match to_value(item) {
        Value::Table(t) => write_asset(head, &t),
        _ => String::new(),
    }
}

/// The start of a new item: a commented file that starts from a section of the game and
/// writes the difference.
#[must_use]
pub fn template(stem_name: &str) -> String {
    let id = gz_characters::model::id_from_name(if stem_name.is_empty() { "new_item" } else { stem_name });
    format!(
        "-- A new item of this module. It starts from an item of the game (`base`) and says what is\n\
-- different; `edit = \"<section>\"` instead of `base` changes an item that exists.\n\
-- `Grizzly-Cmd.exe reference item` lists every key, the linked game's catalog what each means.\n\
return {{\n  item = 1,\n  -- the section in the game is \"<module>_{id}\"; `section = \"...\"` names it\n  base = \"bread\",\n  -- a plain string is the Russian text; {{ rus = \"...\", eng = \"...\" }} gives every language\n  name = \"New item\",\n  description = \"What the player reads about it.\",\n  set = {{ cost = 100, inv_weight = 0.3 }},\n}}\n"
    )
}

/// The keys each level of an item file knows: any other is the author's, kept and noted. The
/// tables of [`fields`] say the same (a test holds them to it).
pub const KNOWN: &[(&str, &[&str])] = &[
    (
        "",
        &[
            "item",
            "id",
            "section",
            "base",
            "abstract",
            "edit",
            "name",
            "short_name",
            "description",
            "icon",
            "set",
            "unset",
            "add",
            "drop",
            "hud",
            "sections",
            "craft",
            "loot",
            "sold",
            "scene",
            "lists",
            "editor",
        ],
    ),
    ("icon", &["texture", "x", "y", "w", "h", "file"]),
    ("hud", &["base", "set", "unset", "add", "drop"]),
    ("sections.*", &["name", "base", "set", "edit", "unset", "add", "drop"]),
    ("craft.*", &["at", "count", "parts"]),
    ("loot", &["count", "chance", "by"]),
    ("sold.*", &["by", "supplies", "count", "chance"]),
    ("scene", &["like", "sound", "camera", "time", "helmet", "anm", "hud_anm", "hud"]),
    ("scene.hud", &["base", "set", "unset", "add", "drop"]),
    ("lists.*", &["file", "section", "key", "value", "add", "drop"]),
];

/// How an item's values map onto its file: new keys in the reference's order.
#[must_use]
pub fn diff_rules() -> DiffRules {
    KNOWN.iter().fold(DiffRules::new(), |rules, (pattern, keys)| rules.order(pattern, keys))
}
