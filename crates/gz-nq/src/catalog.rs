//! The kind catalog: what every node, action and condition kind takes, which pins
//! it has and which document may carry it.
//!
//! The game owns it: `configs\nq\catalog.ltx` of the linked game (the editor's bundled copy when
//! none is linked), the `configs\nq\kinds\*.ltx` beside it, those of every installed module and,
//! last, those of the open project. [`parse_text`] reads one file by a hand-rolled
//! reader does; [`Catalog::assemble`] folds the files in that order - a later file redefining a
//! kind replaces it.

use rustc_hash::FxHashMap;
use serde_json::{Map, Value as Json, json};
use smol_str::SmolStr;

use crate::text::{c_atof, c_atoi, split, trim};

/// A use of a kind.
pub const USE_TRIGGER: u8 = 1;
pub const USE_MAIN: u8 = 2;
pub const USE_EXTRA: u8 = 4;
pub const USE_COND: u8 = 8;

/// The catalog type names: what tells a parameter definition line from a kind attribute that
/// shares its name.
const TYPES: &[&str] = &[
    "string",
    "int",
    "float",
    "bool",
    "enum",
    "text",
    "duration",
    "npc_ref",
    "target_ref",
    "place",
    "spawn_spec",
    "item_list",
    "loot_spawn",
    "spawn_section",
    "item_section",
    "squad_section",
    "level",
    "smart",
    "story_id",
    "profile",
    "community",
    "info",
    "var_name",
    "task_id",
    "quest_id",
    "ref_name",
    "global_name",
    "signal_name",
    "spot_type",
    "relation",
    "lua",
    "cases",
    "cases_cond",
    "cases_weight",
    "kill_target",
    "count_or_all",
    "value",
    "cond_list",
    "node_id",
    "restrictor",
    "squad_ref",
    "object_ref",
    "objective_id",
    "role",
    "way_path",
    "destination",
    "role_or_actor",
    "attack_target",
    "role_bindings",
    "behaviour_id",
    "npc_list",
    "character_id",
    "level_changer",
    "radio_playlist",
];

const ATTRS: &[&str] = &[
    "group", "title", "desc", "use", "docs", "params", "rows", "pins", "wait", "once", "event", "impl",
    "since", "alias", "summary",
];

/// Whether a name is a catalog type.
#[must_use]
pub fn is_type_name(s: &str) -> bool {
    TYPES.contains(&s)
}

/// A parameter of a kind, or a key of one row of its list parameter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Param {
    pub name: String,
    /// The catalog type name (`item_section`, `text`, `enum`, ...).
    pub ty: String,
    pub required: bool,
    /// The default as written in the catalog.
    pub default: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// `enum=a|b|c`.
    pub enums: Vec<String>,
}

fn parse_param(name: &str, spec: &str) -> Param {
    let mut p = Param { name: name.to_owned(), ..Param::default() };
    for (i, t) in split(spec, ',').iter().enumerate() {
        if i == 0 {
            p.ty = t.clone();
            continue;
        }
        if t == "required" {
            p.required = true;
            continue;
        }
        let Some(eq) = t.find('=') else { continue };
        let k = trim(t.get(..eq).unwrap_or(""));
        let v = trim(t.get(eq + 1..).unwrap_or(""));
        match k {
            "default" => p.default = Some(v.to_owned()),
            "min" => p.min = Some(c_atof(v)),
            "max" => p.max = Some(c_atof(v)),
            "enum" => p.enums = split(v, '|'),
            _ => {}
        }
    }
    p
}

/// One kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kind {
    /// `dialog.topic`.
    pub id: String,
    pub group: String,
    pub title: String,
    pub desc: String,
    /// The `USE_*` mask.
    pub uses: u8,
    /// In catalog order.
    pub params: Vec<Param>,
    /// The schema of ONE ROW of a list parameter (`rows` + `row.<name>`).
    pub rows: Vec<Param>,
    /// Declared pins; `cases` means the pins come from `params.cases`.
    pub pins: Vec<String>,
    pub pins_from_cases: bool,
    pub wait: bool,
    pub once_default: bool,
    pub event: bool,
    pub implementation: String,
    pub in_quest: bool,
    pub in_behaviour: bool,
    pub since: i32,
    /// Old names this kind replaces.
    pub alias: Vec<String>,
    /// The parameters that tell a node or an action of the kind apart at a glance, in order
    /// (`summary = text`, `summary = section, count`): what its row on the canvas says.
    pub summary: Vec<String>,
    /// What each pin means (`pin.<name> = ...`): the pin's tooltip.
    pub pin_desc: Vec<(String, String)>,
    /// The file the section came from.
    pub source: String,
}

impl Kind {
    fn new(id: &str, source: &str) -> Self {
        Self {
            id: id.to_owned(),
            source: source.to_owned(),
            in_quest: true,
            in_behaviour: true,
            since: 1,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is(&self, use_: u8) -> bool {
        self.uses & use_ != 0
    }

    /// Whether the document may carry the kind.
    #[must_use]
    pub fn in_doc(&self, behaviour: bool) -> bool {
        if behaviour { self.in_behaviour } else { self.in_quest }
    }

    #[must_use]
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    #[must_use]
    pub fn row_param(&self, name: &str) -> Option<&Param> {
        self.rows.iter().find(|p| p.name == name)
    }

    #[must_use]
    pub fn has_rows(&self) -> bool {
        !self.rows.is_empty()
    }

    /// What a pin means, as the catalog says it.
    #[must_use]
    pub fn pin_text(&self, pin: &str) -> Option<&str> {
        self.pin_desc.iter().find(|(p, _)| p == pin).map(|(_, d)| d.as_str())
    }

    /// A declared (static) pin; `cases` is none.
    #[must_use]
    pub fn has_pin(&self, pin: &str) -> bool {
        self.pins.iter().any(|p| p == pin && p != "cases")
    }
}

/// The `[nq.catalog]` header of the files read so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub version: i32,
    pub api: i32,
    /// The newest format version of a graph the game reads (`format`); 0 when the catalog does
    /// not say, which a game of format 1 did not.
    pub format: i32,
}

fn to_bool(v: &str) -> bool {
    matches!(v, "true" | "1" | "on" | "yes")
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Reads one catalog file into `kinds`. The text is decoded already.
///
/// A hand-rolled reader: a module may redeclare a kind, and a parameter may be
/// named like a kind attribute (`task.set_text: title`), so lines are read in order.
pub fn parse_text(text: &str, source: &str, kinds: &mut Vec<Kind>, header: &mut Header) {
    let mut sect = String::new();
    let mut cur: Option<usize> = None;
    let mut param_names: Vec<String> = Vec::new();
    // the row names are not reset by a new section - a kind without `rows` sees the
    // previous kind's names
    let mut row_names: Vec<String> = Vec::new();
    let mut after_params = false;
    for raw in text.split('\n') {
        let mut line = raw;
        if let Some(c) = line.find(';') {
            line = line.get(..c).unwrap_or("");
        }
        if let Some(c) = line.find("//") {
            line = line.get(..c).unwrap_or("");
        }
        let line = trim(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(close) = line.find(']') else { continue };
            sect = lower(trim(line.get(1..close).unwrap_or("")));
            cur = None;
            param_names.clear();
            after_params = false;
            if sect == "nq.catalog" {
                continue;
            }
            let Some(id) = sect.strip_prefix("nq.") else { continue };
            // a later file redefining a kind wins (module extension)
            if let Some(at) = kinds.iter().position(|k| k.id == id) {
                kinds.remove(at);
            }
            kinds.push(Kind::new(id, source));
            cur = Some(kinds.len() - 1);
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = lower(trim(line.get(..eq).unwrap_or("")));
        let val = trim(line.get(eq + 1..).unwrap_or(""));
        if sect == "nq.catalog" {
            if key == "version" {
                header.version = i32::try_from(c_atoi(val)).unwrap_or(0);
            } else if key == "api" {
                header.api = i32::try_from(c_atoi(val)).unwrap_or(0);
            } else if key == "format" {
                header.format = i32::try_from(c_atoi(val)).unwrap_or(0);
            }
            continue;
        }
        let Some(k) = cur.and_then(|i| kinds.get_mut(i)) else { continue };

        if let Some(pname) = key.strip_prefix("pin.") {
            k.pin_desc.retain(|(p, _)| p != pname);
            k.pin_desc.push((pname.to_owned(), val.to_owned()));
            continue;
        }
        if let Some(rname) = key.strip_prefix("row.") {
            if row_names.iter().any(|n| n == rname) && !k.rows.iter().any(|p| p.name == rname) {
                k.rows.push(parse_param(rname, val));
            }
            continue;
        }

        let in_params = param_names.contains(&key);
        let have_def = k.params.iter().any(|p| p.name == key);
        let first = trim(val.find(',').map_or(val, |c| val.get(..c).unwrap_or("")));
        let looks_like_type = is_type_name(first);
        if in_params && after_params && !have_def && (looks_like_type || !ATTRS.contains(&key.as_str())) {
            let p = parse_param(&key, val);
            let order = |name: &str| param_names.iter().position(|n| n == name).unwrap_or(0);
            let at = order(&key);
            let mut ins = 0;
            while let Some(other) = k.params.get(ins) {
                if order(&other.name) > at {
                    break;
                }
                ins += 1;
            }
            k.params.insert(ins, p);
            continue;
        }
        match key.as_str() {
            "group" => k.group = val.to_owned(),
            "title" => k.title = val.to_owned(),
            "desc" => k.desc = val.to_owned(),
            "use" => {
                k.uses = 0;
                for u in split(val, ',') {
                    k.uses |= match u.as_str() {
                        "trigger" => USE_TRIGGER,
                        "main" => USE_MAIN,
                        "extra" => USE_EXTRA,
                        "cond" => USE_COND,
                        _ => 0,
                    };
                }
            }
            "docs" => {
                // absent = both; naming one narrows it
                let d = split(val, ',');
                k.in_quest = d.is_empty();
                k.in_behaviour = d.is_empty();
                for x in &d {
                    if x == "quest" {
                        k.in_quest = true;
                    } else if x == "behaviour" {
                        k.in_behaviour = true;
                    }
                }
            }
            "params" => {
                param_names = split(val, ',');
                after_params = true;
            }
            "rows" => row_names = split(val, ','),
            "pins" => {
                k.pins = split(val, ',');
                k.pins_from_cases = k.pins.iter().any(|p| p == "cases");
            }
            "wait" => k.wait = to_bool(val),
            "once" => k.once_default = to_bool(val),
            "event" => k.event = to_bool(val),
            "impl" => k.implementation = val.to_owned(),
            "since" => k.since = i32::try_from(c_atoi(val)).unwrap_or(0),
            "alias" => k.alias = split(val, ','),
            "summary" => k.summary = split(val, ','),
            _ => {}
        }
    }
}

/// One file of the catalog: its label (`game:configs\nq\catalog.ltx`, `bundled:...`,
/// `modules\<id>\kinds\<name>`, `project:kinds\<name>.ltx`) and its text, decoded.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub label: String,
    pub text: String,
}

/// The catalog as loaded: immutable, shared by every reader.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    kinds: Vec<Kind>,
    by_id: FxHashMap<SmolStr, usize>,
    by_alias: FxHashMap<SmolStr, usize>,
    /// `game`, `bundled`, or empty when nothing loaded.
    pub source: String,
    pub header: Header,
    /// The files that made it up, in order.
    pub files: Vec<String>,
    /// Why there are no kinds, when there are none.
    pub error: String,
    /// Changes with every load.
    pub generation: u32,
}

impl Catalog {
    /// The newest format version of a graph the game of this catalog reads: what its header
    /// says, 1 when it says nothing; `None` when no catalog was read.
    #[must_use]
    pub fn reads_format(&self) -> Option<i32> {
        (!self.kinds.is_empty()).then(|| self.header.format.max(1))
    }

    /// No kinds: every lookup misses and a trigger is told by its name (`trigger.*`).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Folds the files in order. `from_game` says whether the core catalog is the game's;
    /// `error` is a reason the core file could not be read.
    #[must_use]
    pub fn assemble(files: &[SourceFile], from_game: bool, error: Option<String>, generation: u32) -> Self {
        let mut kinds = Vec::new();
        let mut header = Header::default();
        for f in files {
            parse_text(&f.text, &f.label, &mut kinds, &mut header);
        }
        let mut c = Self::from_kinds(kinds);
        c.header = header;
        c.files = files.iter().map(|f| f.label.clone()).collect();
        c.generation = generation;
        if let Some(e) = error {
            c.error = e;
            return c;
        }
        c.source = (if from_game { "game" } else { "bundled" }).to_owned();
        if c.kinds.is_empty() {
            c.error = "the catalog declares no kinds".to_owned();
        }
        c
    }

    /// A catalog of these kinds (tests, tools).
    #[must_use]
    pub fn from_kinds(kinds: Vec<Kind>) -> Self {
        let mut by_id = FxHashMap::default();
        let mut by_alias = FxHashMap::default();
        for (i, k) in kinds.iter().enumerate() {
            by_id.entry(SmolStr::new(&k.id)).or_insert(i);
        }
        for (i, k) in kinds.iter().enumerate() {
            for a in &k.alias {
                by_alias.entry(SmolStr::new(a)).or_insert(i);
            }
        }
        Self { kinds, by_id, by_alias, ..Self::default() }
    }

    /// Whether it holds any kind.
    #[must_use]
    pub fn loaded(&self) -> bool {
        !self.kinds.is_empty()
    }

    #[must_use]
    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    /// A kind by its id, or by an old name it replaces.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&Kind> {
        self.by_id.get(id).or_else(|| self.by_alias.get(id)).and_then(|&i| self.kinds.get(i))
    }

    /// Whether a kind roots the graph: the catalog's `use = trigger` without `main`, the name
    /// when the catalog does not know it.
    #[must_use]
    pub fn is_trigger(&self, kind: &str) -> bool {
        if kind.is_empty() {
            return false;
        }
        match self.find(kind) {
            Some(k) => k.is(USE_TRIGGER) && !k.is(USE_MAIN),
            None => crate::text::is_trigger_kind(kind),
        }
    }

    /// The kinds usable at a position, in catalog order, of one document kind.
    #[must_use]
    pub fn kinds_for(&self, use_mask: u8, behaviour: bool) -> Vec<&Kind> {
        self.kinds.iter().filter(|k| k.uses & use_mask != 0 && k.in_doc(behaviour)).collect()
    }

    /// `asset.kinds`'s reply fields, or the error.
    pub fn to_json(&self) -> Result<Json, String> {
        if !self.loaded() {
            return Err(if self.error.is_empty() {
                "catalog not available".to_owned()
            } else {
                self.error.clone()
            });
        }
        let kinds: Vec<Json> = self.kinds.iter().map(kind_json).collect();
        Ok(json!({
            "source": self.source,
            "version": self.header.version,
            "api": self.header.api,
            "format": self.reads_format(),
            "files": self.files,
            "count": self.kinds.len(),
            "kinds": kinds,
        }))
    }
}

fn number(n: f64) -> Json {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { json!(n as i64) } else { json!(n) }
}

fn params_json(list: &[Param]) -> Json {
    Json::Array(
        list.iter()
            .map(|p| {
                let mut o = Map::new();
                o.insert("name".into(), json!(p.name));
                o.insert("type".into(), json!(p.ty));
                o.insert("required".into(), json!(p.required));
                if let Some(d) = &p.default {
                    o.insert("default".into(), json!(d));
                }
                if let Some(m) = p.min {
                    o.insert("min".into(), number(m));
                }
                if let Some(m) = p.max {
                    o.insert("max".into(), number(m));
                }
                if !p.enums.is_empty() {
                    o.insert("enum".into(), json!(p.enums));
                }
                Json::Object(o)
            })
            .collect(),
    )
}

fn kind_json(k: &Kind) -> Json {
    let mut o = Map::new();
    o.insert("id".into(), json!(k.id));
    o.insert("group".into(), json!(k.group));
    o.insert("title".into(), json!(k.title));
    o.insert("desc".into(), json!(k.desc));
    let mut uses = Vec::new();
    for (bit, name) in
        [(USE_TRIGGER, "trigger"), (USE_MAIN, "main"), (USE_EXTRA, "extra"), (USE_COND, "cond")]
    {
        if k.is(bit) {
            uses.push(name);
        }
    }
    o.insert("use".into(), json!(uses));
    o.insert("params".into(), params_json(&k.params));
    if !k.rows.is_empty() {
        o.insert("rows".into(), params_json(&k.rows));
    }
    if k.is(USE_MAIN | USE_TRIGGER) {
        o.insert("pins".into(), json!(k.pins));
        o.insert("wait".into(), json!(k.wait));
        o.insert("once".into(), json!(k.once_default));
    }
    if k.is(USE_COND) {
        o.insert("event".into(), json!(k.event));
    }
    o.insert("impl".into(), json!(k.implementation));
    o.insert("since".into(), json!(k.since));
    if !k.alias.is_empty() {
        o.insert("alias".into(), json!(k.alias));
    }
    if !k.summary.is_empty() {
        o.insert("summary".into(), json!(k.summary));
    }
    if !k.pin_desc.is_empty() {
        let pins: Map<String, Json> = k.pin_desc.iter().map(|(p, d)| (p.clone(), json!(d))).collect();
        o.insert("pin_desc".into(), Json::Object(pins));
    }
    o.insert("source".into(), json!(k.source));
    Json::Object(o)
}

/// The groups of parameters a kind takes only one of at a time; each entry is a comma separated
/// list of names. The catalog line cannot say this yet.
#[must_use]
pub fn exclusive_forms(kind: &str) -> &'static [&'static str] {
    match kind {
        "objective.fetch" | "item.spawn" | "item.give" | "item.take" => &["section,count", "items"],
        "objective.kill_count" => &["community", "squad", "section"],
        _ => &[],
    }
}

/// The index of the form `param` belongs to.
#[must_use]
pub fn form_index(kind: &str, param: &str) -> Option<usize> {
    exclusive_forms(kind).iter().position(|f| f.split(',').any(|p| p == param))
}

/// Whether a value already present rules out the form `param` belongs to.
#[must_use]
pub fn form_ruled_out(kind: &str, param: &str, params: &gz_luadata::Value) -> bool {
    let Some(mine) = form_index(kind, param) else { return false };
    params.fields().iter().any(|(k, _)| form_index(kind, k).is_some_and(|other| other != mine))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "[nq.catalog]\nversion = 21\napi = 1\n\n[nq.trigger.start]\ngroup = triggers\ntitle = Quest start ; a comment\nuse = trigger\nparams =\npins = next\nonce = true\n\n[nq.task.set_text]\nuse = extra\nparams = task, title\ntask = task_id, required\ntitle = text\n\n[nq.item.spawn]\nuse = extra\nparams = items\nitems = item_list\nrows = section, count\nrow.section = item_section\nrow.count = int, default=1, min=1\n";

    #[test]
    fn reads_kinds_params_and_rows() {
        let mut kinds = Vec::new();
        let mut h = Header::default();
        parse_text(ONE, "bundled:configs\\nq\\catalog.ltx", &mut kinds, &mut h);
        assert_eq!(h, Header { version: 21, api: 1, format: 0 });
        assert_eq!(kinds.len(), 3);
        let mut c = Catalog::from_kinds(kinds);
        // a game whose catalog says no format reads format 1
        assert_eq!(c.reads_format(), Some(1));
        c.header.format = 2;
        assert_eq!(c.reads_format(), Some(2));
        assert_eq!(Catalog::empty().reads_format(), None);
        let start = c.find("trigger.start").map(|k| (k.title.clone(), k.once_default, k.pins.clone()));
        assert_eq!(start, Some(("Quest start".to_owned(), true, vec!["next".to_owned()])));
        assert!(c.is_trigger("trigger.start"));
        let st = c
            .find("task.set_text")
            .map(|k| k.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect::<Vec<_>>());
        assert_eq!(
            st,
            Some(vec![("task".to_owned(), "task_id".to_owned()), ("title".to_owned(), "text".to_owned())])
        );
        let rows = c.find("item.spawn").map(|k| k.rows.len());
        assert_eq!(rows, Some(2));
        assert_eq!(c.find("item.spawn").and_then(|k| k.row_param("count")).and_then(|p| p.min), Some(1.0));
    }

    #[test]
    fn later_files_win_and_move_the_kind_last() {
        let a = SourceFile {
            label: "a".into(),
            text: "[nq.x]\ntitle = A\nuse = main\n[nq.y]\nuse = main\n".into(),
        };
        let b = SourceFile { label: "b".into(), text: "[nq.x]\ntitle = B\nuse = extra\n".into() };
        let c = Catalog::assemble(&[a, b], true, None, 1);
        let ids: Vec<&str> = c.kinds().iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, ["y", "x"]);
        assert_eq!(c.find("x").map(|k| (k.title.as_str(), k.source.as_str())), Some(("B", "b")));
        assert_eq!(c.source, "game");
        assert_eq!(c.files, ["a", "b"]);
    }

    #[test]
    fn forms() {
        assert_eq!(form_index("objective.fetch", "count"), Some(0));
        assert_eq!(form_index("objective.fetch", "items"), Some(1));
        assert_eq!(form_index("objective.fetch", "from"), None);
        let mut p = gz_luadata::Value::table();
        p.set("items", gz_luadata::Value::table());
        assert!(form_ruled_out("objective.fetch", "section", &p));
        assert!(!form_ruled_out("objective.fetch", "items", &p));
    }

    #[test]
    fn json_and_missing() {
        assert_eq!(Catalog::empty().to_json().err().as_deref(), Some("catalog not available"));
        let c = Catalog::assemble(&[SourceFile { label: "l".into(), text: ONE.into() }], false, None, 3);
        let j = c.to_json().unwrap_or_default();
        assert_eq!(j["source"], "bundled");
        assert_eq!(j["count"], 3);
        assert_eq!(j["kinds"][0]["use"][0], "trigger");
    }

    /// The bundled catalog's `summary` names parameters of its kind, and its pin texts name pins
    /// the kind has: what the canvas prints and its pins' tooltips.
    #[test]
    fn the_bundled_summaries_and_pin_texts_name_what_the_kind_has() {
        let bytes = include_bytes!("../../../gamedata/configs/nq/catalog.ltx");
        let text = gz_base::text::cp1251_to_utf8(bytes).into_owned();
        let c = Catalog::assemble(&[SourceFile { label: "bundled".into(), text }], false, None, 1);
        let mut with_summary = 0;
        for k in c.kinds() {
            for name in &k.summary {
                assert!(k.param(name).is_some(), "{}: summary names '{name}', no parameter of it", k.id);
            }
            with_summary += usize::from(!k.summary.is_empty());
            for (pin, desc) in &k.pin_desc {
                assert!(k.has_pin(pin), "{}: pin.{pin} names no pin of it", k.id);
                assert!(!desc.is_empty());
            }
        }
        // every kind with parameters says which of them tells it apart
        let unsaid: Vec<&str> = c
            .kinds()
            .iter()
            .filter(|k| !k.params.is_empty() && k.summary.is_empty())
            .map(|k| k.id.as_str())
            .collect();
        assert!(unsaid.is_empty(), "no summary: {unsaid:?}");
        assert!(with_summary > 100);
        assert_eq!(
            c.find("wait.when").and_then(|k| k.pin_text("timeout")),
            Some("When the timeout runs out first")
        );
    }
}
