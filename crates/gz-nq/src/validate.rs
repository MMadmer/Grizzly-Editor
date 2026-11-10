//! The validator (`NqValidate`): one rule list with the codes the game runtime prints; errors
//! block the build, warnings do not. A pure function of the graph, the catalog and what the
//! [`Context`] says about the project and the linked game.

use gz_luadata::Value;
use rustc_hash::FxHashSet;
use serde_json::{Value as Json, json};
use smol_str::SmolStr;

use crate::asset::{Action, Cond, Graph, Node, cond_from_value, is_behaviour_path, is_quest_path};
use crate::caps::{self, ItemCaps};
use crate::catalog::{self, Catalog, Kind, Param, USE_COND, USE_EXTRA, USE_MAIN, USE_TRIGGER};
use crate::pickers::{PickerType, Pickers};
use crate::text::{self, cp1251_covers, id_from_name, split, trim, valid_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// One problem: `CODE node[slot]: message` (`SNqProblem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NqProblem {
    pub severity: Severity,
    /// `E007`, `W011`, ...
    pub code: SmolStr,
    /// Empty: the graph itself.
    pub node: String,
    /// `enter:2`, `cond:1`, `param:npc`, `out:next`, `case:1`, or empty.
    pub slot: String,
    pub message: String,
}

impl NqProblem {
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// `E007 node[slot]: message`.
    #[must_use]
    pub fn text(&self) -> String {
        let mut s = self.code.to_string();
        if !self.node.is_empty() {
            s.push(' ');
            s.push_str(&self.node);
        }
        if !self.slot.is_empty() {
            s.push('[');
            s.push_str(&self.slot);
            s.push(']');
        }
        s.push_str(": ");
        s.push_str(&self.message);
        s
    }

    #[must_use]
    pub fn to_json(&self) -> Json {
        json!({
            "code": self.code,
            "severity": if self.is_error() { "error" } else { "warning" },
            "node": self.node,
            "slot": self.slot,
            "message": self.message,
        })
    }
}

/// The reader's shape problems as `E003` (`ShapeErrors`).
#[must_use]
pub fn shape_errors(shape: &[String]) -> Vec<NqProblem> {
    shape
        .iter()
        .map(|m| NqProblem { severity: Severity::Error, code: "E003".into(), node: String::new(), slot: String::new(), message: m.clone() })
        .collect()
}

#[must_use]
pub fn count_errors(v: &[NqProblem]) -> usize {
    v.iter().filter(|p| p.is_error()).count()
}

#[must_use]
pub fn count_warnings(v: &[NqProblem]) -> usize {
    v.iter().filter(|p| !p.is_error()).count()
}

#[must_use]
pub fn to_json(v: &[NqProblem]) -> Json {
    Json::Array(v.iter().map(NqProblem::to_json).collect())
}

/// `problems: none`, or `problems:` and one line per problem (`ToText`).
#[must_use]
pub fn to_text(v: &[NqProblem]) -> String {
    if v.is_empty() {
        return "problems: none".to_owned();
    }
    let mut s = String::from("problems:");
    for p in v {
        s.push('\n');
        s.push_str(&p.text());
    }
    s
}

/// A character of the project as the validator and the pickers see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterBrief {
    /// The `id` of its `.npcasset`.
    pub id: String,
    /// A template (`abstract = true`): never an NPC.
    pub template: bool,
    /// What the author reads.
    pub name: String,
    /// The file, project-relative.
    pub file: String,
}

/// What the validator knows besides the graph (`SContext`).
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    /// The document's file, for `E001` and `W061`; `None` also skips the project's characters
    /// and global variables.
    pub file_path: Option<&'a str>,
    /// The ids of the OTHER graphs of the project (`E002`, a bare `quest_id`).
    pub project_quest_ids: Option<&'a [String]>,
    /// The ids of the project's behaviour graphs: what `behaviour.run` may name.
    pub project_behaviour_ids: Option<&'a [String]>,
    /// The level of the open scene when the linked game knows it (`E031`).
    pub level: Option<&'a str>,
    /// The linked game's lookups (`W060`); `None` without a linked game.
    pub pickers: Option<&'a Pickers>,
    /// What the linked game's sections can carry (`W062`, `W063`).
    pub caps: Option<&'a ItemCaps>,
    pub characters: &'a [CharacterBrief],
    /// The module's global variables (`mod.ltx [vars]`).
    pub globals: &'a [String],
}

struct RefSite {
    reference: String,
    node: Option<usize>,
    /// cond: i, on_enter: 100+i, main: 200, on_exit: 300+i.
    order: i32,
    slot: String,
    /// Declarations: what made it - `squad`, `object`, `item`.
    kind: &'static str,
}

struct V<'a> {
    q: &'a Graph,
    cat: &'a Catalog,
    ctx: &'a Context<'a>,
    out: Vec<NqProblem>,
    catalog: bool,
    pickers: Option<&'a Pickers>,
    decls: Vec<RefSite>,
    item_uses: Vec<RefSite>,
    uses: Vec<RefSite>,
}

fn is_integer(d: f64) -> bool {
    d == d.floor() && d.abs() < 1e15
}

fn slot_name(prefix: &str, param: &str) -> String {
    let mut s = prefix.to_owned();
    if !param.is_empty() {
        if !s.is_empty() {
            s.push('/');
        }
        s.push_str("param:");
        s.push_str(param);
    }
    s
}

fn g(v: f64) -> String {
    gz_luadata::format_g(v, 6)
}

fn str_of(v: &Value) -> Option<&str> {
    v.as_str()
}

impl<'a> V<'a> {
    fn add(&mut self, sev: Severity, code: &str, node: Option<&str>, slot: &str, message: String) {
        self.out.push(NqProblem { severity: sev, code: code.into(), node: node.unwrap_or("").to_owned(), slot: slot.to_owned(), message });
    }

    fn err(&mut self, code: &str, node: Option<&str>, slot: &str, message: String) {
        self.add(Severity::Error, code, node, slot, message);
    }

    fn warn(&mut self, code: &str, node: Option<&str>, slot: &str, message: String) {
        self.add(Severity::Warning, code, node, slot, message);
    }

    fn node_id(&self, i: Option<usize>) -> Option<&'a str> {
        i.and_then(|i| self.q.nodes.get(i)).map(|n| n.id.as_str())
    }

    // ---- E022 --------------------------------------------------------------------------------

    fn check_kind_form(&mut self, kind: &str, params: &Value, node: Option<&str>, slot: &str) {
        let forms = catalog::exclusive_forms(kind);
        if forms.is_empty() || !params.is_table() {
            return;
        }
        // the first parameter of each form, in the order the file lists them
        let mut picked: Vec<Option<&str>> = vec![None; forms.len()];
        for (k, _) in params.fields() {
            if let Some(f) = catalog::form_index(kind, k)
                && let Some(p) = picked.get_mut(f)
                && p.is_none()
            {
                *p = Some(k.as_str());
            }
        }
        let used = picked.iter().filter(|p| p.is_some()).count();
        if used > 1 {
            let list = picked.iter().flatten().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(" and ");
            self.err("E022", node, slot, format!("{kind} takes only one of these at a time, {list} are set together"));
        }
        if used == 0 && matches!(kind, "objective.fetch" | "item.spawn" | "item.give" | "item.take") {
            self.err("E022", node, slot, format!("{kind} needs an items list (or its one-line spelling, section)"));
        }
        if matches!(kind, "objective.fetch" | "objective.kill" | "objective.reach") {
            if params.has("map_spot") || params.has("spot_text") {
                self.err("E006", node, "", "map_spot/spot_text are gone - bind the marker with spot_task (and spot_objective for a step)".into());
            }
            if params.has("spot_objective") && !params.has("spot_task") {
                self.err("E022", node, "", "spot_objective needs spot_task".into());
            }
            if params.has("spot_done") && !params.has("spot_task") {
                self.err("E022", node, "", "spot_done needs spot_task".into());
            }
        }
        if kind == "objective.fetch" {
            if params.has("item") {
                self.err("E006", node, "", "the separate 'item' parameter is gone - use an items row { item = { ref = ... } }".into());
            }
            if params.has("from") && params.has("spawn") {
                self.err("E022", node, "", "spawn already decides where the loot is - from would watch a second container".into());
            }
            if params.has("count") && params.has("items") {
                self.err("E022", node, "", "count belongs to section - the items list carries its own counts".into());
            }
            if params.has("spawn")
                && let Some(rows) = params.get("items")
                && rows.arr().iter().any(|r| r.is_table() && r.has("item"))
            {
                self.err("E022", node, "", "spawn creates its own instances - a concrete-object row cannot be combined with it".into());
            }
        }
    }

    // ---- W060 --------------------------------------------------------------------------------

    fn check_indexed(&mut self, t: Option<PickerType>, value: &str, node: Option<&str>, slot: &str) {
        let (Some(p), Some(t)) = (self.pickers, t) else { return };
        if value.is_empty() || p.entries(t).is_empty() || p.known(t, value) {
            return;
        }
        // restrictors are the one index the project fills, not the game
        if t == PickerType::Restrictor {
            self.warn("W060", node, slot, format!("'{value}' is not a restrictor placed in this project's levels"));
        } else {
            self.warn("W060", node, slot, format!("'{value}' is not a known {} of the linked game", t.name()));
        }
    }

    // ---- W040 --------------------------------------------------------------------------------

    fn check_text(&mut self, v: &Value, node: Option<&str>, slot: &str) {
        match v {
            Value::String(s) => {
                let (ok, bad) = cp1251_covers(s);
                if !ok {
                    self.warn("W040", node, slot, format!("text has characters outside cp1251 ({bad}) - the game will show '?'"));
                }
            }
            Value::Table(t) => {
                for (_, x) in &t.fields {
                    self.check_text(x, node, slot);
                }
            }
            _ => {}
        }
    }

    // ---- references ----------------------------------------------------------------------------

    fn note_ref_use(&mut self, tbl: &Value, node: Option<usize>, order: i32, slot: &str) {
        if let Some(Value::String(r)) = tbl.get("ref")
            && !r.is_empty()
        {
            self.uses.push(RefSite { reference: r.to_string(), node, order, slot: slot.to_owned(), kind: "" });
        }
    }

    fn note_ref_decl(&mut self, name: &str, node: Option<usize>, order: i32, slot: &str, kind: &'static str) {
        if name.is_empty() {
            return;
        }
        self.decls.push(RefSite { reference: name.to_owned(), node, order, slot: slot.to_owned(), kind });
    }

    fn note_item_use(&mut self, v: &Value, node: Option<usize>, order: i32, slot: &str) {
        if !v.is_table() || !v.has("ref") {
            return;
        }
        let r = v.get_string("ref", "");
        if !r.is_empty() {
            self.item_uses.push(RefSite { reference: r, node, order, slot: slot.to_owned(), kind: "" });
        }
    }

    fn check_character_name(&mut self, ch: Option<&Value>, node: Option<&str>, slot: &str) {
        let name = match ch {
            Some(Value::String(s)) if !s.is_empty() => s.as_str(),
            _ => {
                self.err("E006", node, slot, "character must be the id of a character of the project".into());
                return;
            }
        };
        if name.contains('.') || self.ctx.file_path.is_none() {
            return;
        }
        match self.ctx.characters.iter().rev().find(|b| b.id == name) {
            None => self.err("E030", node, slot, format!("character '{name}' is not a character of this project")),
            Some(b) if b.template => self.err("E030", node, slot, format!("character '{name}' is a template - it never becomes an NPC")),
            Some(_) => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_npc_ref(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, allow_smart: bool, allow_spawn: bool) -> bool {
        if !v.is_table() {
            self.err(
                "E006",
                node,
                slot,
                format!(
                    "expected a reference table {{ story = ... }} / {{ ref = ... }} / {{ character = ... }} / {{ profile = ... }} / {{ community = ... }}{}",
                    if allow_smart { " / { smart = ... }" } else { "" }
                ),
            );
            return false;
        }
        let mut alts = 0;
        if v.has("story") {
            alts += 1;
            self.check_indexed(Some(PickerType::Story), &v.get_string("story", ""), node, slot);
        }
        if v.has("ref") {
            alts += 1;
            self.note_ref_use(v, nidx, order, slot);
        }
        if v.has("character") {
            alts += 1;
            self.check_character_name(v.get("character"), node, slot);
        }
        if v.has("profile") {
            alts += 1;
            self.check_indexed(Some(PickerType::Profile), &v.get_string("profile", ""), node, slot);
        }
        if v.has("community") {
            alts += 1;
            self.check_indexed(Some(PickerType::Community), &v.get_string("community", ""), node, slot);
            if v.has("level") {
                self.check_indexed(Some(PickerType::Level), &v.get_string("level", ""), node, slot);
            }
        }
        if allow_smart && v.has("smart") {
            alts += 1;
            self.check_indexed(Some(PickerType::Smart), &v.get_string("smart", ""), node, slot);
        }
        if allow_smart && v.has("restrictor") {
            alts += 1;
            if v.get("restrictor").is_some_and(|r| !r.is_string()) {
                self.err("E006", node, slot, "restrictor must be a string".into());
            } else {
                self.check_indexed(Some(PickerType::Restrictor), &v.get_string("restrictor", ""), node, slot);
            }
        }
        if allow_smart && v.has("pos") {
            alts += 1;
            if !v.get("pos").is_some_and(|p| p.is_table() && p.arr().len() == 3) {
                self.err("E006", node, slot, "pos must be { x, y, z }".into());
            }
            if v.has("level") {
                self.check_indexed(Some(PickerType::Level), &v.get_string("level", ""), node, slot);
            } else {
                self.warn("W060", node, slot, "a { pos } target without a level is placed on the level the quest runs on".into());
            }
        }
        if allow_spawn && let Some(sp) = v.get("spawn") {
            alts += 1;
            self.check_spawn_spec(sp, node, slot, nidx, order, "spawn");
        }
        if alts == 0 {
            self.err(
                "E006",
                node,
                slot,
                format!(
                    "reference table needs one of story/ref/character/profile/community{}{}",
                    if allow_smart { "/smart" } else { "" },
                    if allow_spawn { "/spawn" } else { "" }
                ),
            );
            return false;
        }
        if alts > 1 {
            self.err("E006", node, slot, "reference table must name exactly one target kind".into());
        }
        for (k, x) in v.fields() {
            if k == "level" || k == "pos" || k == "radius" {
                continue;
            }
            if !x.is_string() && k != "spawn" {
                self.err("E006", node, slot, format!("reference field '{k}' must be a string"));
            }
        }
        true
    }

    fn check_object_ref(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, ty: &str) {
        let object = ty == "object_ref";
        if !v.is_table() {
            self.err("E006", node, slot, format!("expected {ty} = {{ story = ... }} or {{ ref = ... }}{}", if object { " or { character = ... }" } else { "" }));
            return;
        }
        let mut alts = 0;
        if v.has("story") {
            alts += 1;
            self.check_indexed(Some(PickerType::Story), &v.get_string("story", ""), node, slot);
        }
        if v.has("ref") {
            alts += 1;
            self.note_ref_use(v, nidx, order, slot);
        }
        if object && v.has("character") {
            alts += 1;
            self.check_character_name(v.get("character"), node, slot);
        }
        if alts != 1 {
            self.err("E006", node, slot, format!("{ty} needs exactly one of story / ref{}", if object { " / character" } else { "" }));
        }
        for (k, x) in v.fields() {
            if k != "story" && k != "ref" && !(object && k == "character") {
                self.err("E006", node, slot, format!("{ty} has no field '{k}'"));
            } else if !x.is_string() {
                self.err("E006", node, slot, format!("'{k}' must be a string"));
            }
        }
    }

    fn check_spawn_spec(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, what: &str) {
        // an empty smart is not a smart: the runtime reads it as unset, so must this
        let smart = v.get_string("smart", "");
        if !v.is_table() || !v.has("section") || (smart.is_empty() && !v.has("place")) {
            self.err("E006", node, slot, format!("'{what}' needs {{ section = ... }} and a smart or a place to put them on"));
            return;
        }
        self.check_indexed(Some(PickerType::Squad), &v.get_string("section", ""), node, slot);
        self.check_indexed(Some(PickerType::Smart), &smart, node, slot);
        if let Some(pl) = v.get("place") {
            self.check_place(pl, node, slot, nidx, order);
        }
        if v.has("restrictor") {
            self.check_indexed(Some(PickerType::Restrictor), &v.get_string("restrictor", ""), node, slot);
        }
        self.note_ref_decl(&v.get_string("ref", ""), nidx, order, slot, "squad");
        if let Some(Value::Bool(false)) = v.get("hold") {
            self.warn("W031", node, slot, "spawned squad without hold - it may leave for the simulation".into());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_row_state(&mut self, e: &Value, k: &Kind, sect: &str, node: Option<&str>, sl: &str, nidx: Option<usize>, order: i32) {
        let caps = self.ctx.caps.map_or_else(|| std::sync::Arc::new(caps::Caps::default()), |c| c.of(sect));
        for (key, val) in e.fields() {
            if key == "section" || key == "item" {
                continue;
            }
            let Some(rp) = k.row_param(key) else {
                let names = k.rows.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
                self.err("E006", node, sl, format!("unknown row key '{key}' - a spawn row takes {names}"));
                continue;
            };
            self.check_param_value(k, rp, val, node, sl, nidx, order, false, Some(e));
            let need = caps::cap_of_key(key);
            if need != caps::ALL && caps.known && !caps.has(need) {
                self.warn("W062", node, sl, format!("{} does not apply to {sect} - {}", caps::field_name(need), caps::why(need)));
            }
        }
        if let Some(Value::Number(a)) = e.get("ammo") {
            if caps.has(caps::AMMO_BOX) && caps.box_size > 0 && *a > f64::from(caps.box_size) {
                self.warn("W063", node, sl, format!("ammo {} is over the box of {sect} ({})", *a as i32, caps.box_size));
            } else if !caps.has(caps::AMMO_BOX) && caps.mag_size > 0 && *a > f64::from(caps.mag_size) {
                self.warn("W063", node, sl, format!("ammo {} is over the magazine of {sect} ({})", *a as i32, caps.mag_size));
            }
        }
        if let Some(Value::String(ak)) = e.get("ammo_kind")
            && !ak.is_empty()
            && caps.known
            && !caps.ammo_class.is_empty()
            && !caps.ammo_class.iter().any(|c| c == ak.as_str())
        {
            self.err("E006", node, sl, format!("{sect} does not chamber '{ak}' - its ammo_class is {}", caps.ammo_class.join(", ")));
        }
        if let Some(Value::String(up)) = e.get("upgrades") {
            let mut seen: Vec<String> = Vec::new();
            for id in split(up, ',') {
                let id = trim(&id).to_owned();
                if id.is_empty() {
                    continue;
                }
                // the engine stops on a duplicate upgrade, and no pcall catches that
                if seen.contains(&id) {
                    self.err("E006", node, sl, format!("upgrade '{id}' is listed twice - the game aborts on a duplicate upgrade"));
                }
                seen.push(id.clone());
                if caps.known && !caps.upgrades.is_empty() && !caps.upgrades.contains(&id) {
                    self.err("E006", node, sl, format!("upgrade '{id}' does not belong to {sect} - it would crash on spawn"));
                }
            }
        }
        // a ref names ONE object
        if let Some(Value::String(r)) = e.get("ref")
            && !r.is_empty()
        {
            let n = e.get_number("count", 1.0) as i32;
            if n > 1 {
                self.err("E006", node, sl, format!("ref names one object - this row makes {n} items"));
            } else {
                self.note_ref_decl(r, nidx, order, sl, "item");
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_item_list(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, objects_ok: bool, rows_of: Option<&Kind>) {
        if !v.is_table() || v.arr().is_empty() {
            if objects_ok {
                self.err("E006", node, slot, "items must be a non-empty list of rows: { section = ..., count = N } or { item = { ref/story } }".into());
            } else {
                self.err("E006", node, slot, "items must be a non-empty list of rows: { section = ..., count = N }".into());
            }
            return;
        }
        let mut seen: Vec<String> = Vec::new();
        for (i, e) in v.arr().iter().enumerate() {
            let sl = format!("{slot}/item:{i}");
            if !e.is_table() {
                self.err("E006", node, &sl, "a row must be a table".into());
                continue;
            }
            let has_section = !e.get_string("section", "").is_empty();
            let has_item = e.has("item");
            if has_section == has_item {
                self.err("E006", node, &sl, format!("a row is either a section or a concrete object, not {}", if has_item { "both" } else { "neither" }));
                continue;
            }
            if let Some(item) = e.get("item") {
                if !objects_ok {
                    self.err("E006", node, &sl, "a concrete object cannot be spawned - this list takes sections only".into());
                    continue;
                }
                if e.has("count") {
                    self.err("E006", node, &sl, "a concrete object IS one - it takes no count".into());
                }
                self.check_object_ref(item, node, &sl, nidx, order, "object_ref");
                self.note_item_use(item, nidx, order, &sl);
                continue;
            }
            let sect = e.get_string("section", "");
            if seen.contains(&sect) {
                self.err("E006", node, &sl, format!("section '{sect}' is listed twice - the counts would be ambiguous"));
            }
            seen.push(sect.clone());
            self.check_indexed(Some(PickerType::Item), &sect, node, &sl);
            if !rows_of.is_some_and(|k| k.row_param("count").is_some())
                && let Some(n) = e.get("count")
                && !n.as_number().is_some_and(|n| n >= 1.0)
            {
                self.err("E006", node, &sl, "count must be an integer >= 1".into());
            }
            match rows_of {
                Some(k) => self.check_row_state(e, k, &sect, node, &sl, nidx, order),
                None => {
                    // the rows of objective.fetch say what the player must BRING
                    for (key, _) in e.fields() {
                        if key != "section" && key != "count" && key != "item" {
                            self.err("E006", node, &sl, format!("unknown row key '{key}' - a row here takes section and count"));
                        }
                    }
                }
            }
        }
    }

    fn check_loot_spawn(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32) {
        const KEYS: [&str; 6] = ["place", "into", "stash", "npc", "corpse", "new_corpse"];
        if !v.is_table() {
            self.err("E006", node, slot, "spawn must be a table with one of place | into | stash | npc | corpse | new_corpse".into());
            return;
        }
        let used = KEYS.iter().filter(|k| v.has(k)).count();
        if used != 1 {
            self.err("E006", node, slot, format!("spawn needs exactly one of place | into | stash | npc | corpse | new_corpse ({used} set)"));
            return;
        }
        if let Some(pl) = v.get("place") {
            self.check_place(pl, node, slot, nidx, order);
        }
        if let Some(into) = v.get("into") {
            self.check_object_ref(into, node, slot, nidx, order, "object_ref");
            self.note_item_use(into, nidx, order, slot);
        }
        if let Some(np) = v.get("npc") {
            self.check_npc_ref(np, node, slot, nidx, order, false, false);
        }
        for which in [false, true] {
            let what = if which { "corpse" } else { "stash" };
            let Some(sel) = v.get(what) else { continue };
            if !sel.is_table() {
                self.err("E006", node, slot, format!("{what} must be a table of filters"));
                continue;
            }
            let pick = sel.get_string("pick", "");
            if !pick.is_empty() && pick != "random" && pick != "nearest" && pick != "farthest" {
                self.err("E006", node, slot, "pick must be random | nearest | farthest".into());
            }
            let (mut within, mut beyond) = (0.0, 0.0);
            if let Some(w) = sel.get("within") {
                match w.as_number() {
                    Some(n) if n > 0.0 => within = n,
                    _ => self.err("E006", node, slot, "within must be a radius > 0".into()),
                }
            }
            if let Some(b) = sel.get("beyond") {
                match b.as_number() {
                    Some(n) if n > 0.0 => beyond = n,
                    _ => self.err("E006", node, slot, "beyond must be a radius > 0".into()),
                }
            }
            if within > 0.0 && beyond > 0.0 && within <= beyond {
                self.warn("W031", node, slot, format!("within {within:.0} <= beyond {beyond:.0} leaves an empty ring - the fallback will decide every time"));
            }
            if !which {
                let st = sel.get_string("state", "");
                if !st.is_empty() && st != "any" && st != "full" && st != "empty" {
                    self.err("E006", node, slot, "state must be any | full | empty".into());
                }
            } else if sel.has("state") || sel.has("scatter") {
                self.err("E006", node, slot, "corpse filters are pick / within / beyond only".into());
            }
        }
        if let Some(nc) = v.get("new_corpse") {
            if !nc.is_table() || nc.get_string("section", "").is_empty() {
                self.err("E006", node, slot, "new_corpse needs { section = ..., place = ... }".into());
            } else {
                self.check_indexed(Some(PickerType::SpawnSection), &nc.get_string("section", ""), node, slot);
                match nc.get("place") {
                    Some(pl) => {
                        self.check_place(pl, node, slot, nidx, order);
                    }
                    None => self.err("E006", node, slot, "new_corpse needs a place to lie in".into()),
                }
            }
        }
    }

    fn check_place(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32) -> bool {
        if !v.is_table() {
            self.err(
                "E006",
                node,
                slot,
                "expected a place { level = ..., pos = { x, y, z }, radius = r } / { restrictor = ... } / { smart = ... } / { ref = ... } / { actor = true, distance = m }".into(),
            );
            return false;
        }
        // relative to the player: resolved at run time, nothing to look up
        if let Some(a) = v.get("actor") {
            if a.as_bool() != Some(true) {
                self.err("E006", node, slot, "actor must be true: { actor = true, distance = metres ahead of the player }".into());
            }
            if let Some(d) = v.get("distance")
                && !d.as_number().is_some_and(|n| n >= 0.0)
            {
                self.err("E006", node, slot, "distance must be a number of metres >= 0".into());
            }
            return true;
        }
        if let Some(r) = v.get("ref") {
            if !r.is_string() {
                self.err("E006", node, slot, "ref must be a string".into());
            } else {
                self.note_ref_use(v, nidx, order, slot);
            }
            return true;
        }
        if let Some(r) = v.get("restrictor") {
            if !r.is_string() {
                self.err("E006", node, slot, "restrictor must be a string".into());
            } else {
                self.check_indexed(Some(PickerType::Restrictor), &v.get_string("restrictor", ""), node, slot);
            }
            return true;
        }
        if v.has("smart") {
            self.check_indexed(Some(PickerType::Smart), &v.get_string("smart", ""), node, slot);
            return true;
        }
        if v.has("level") || v.has("pos") {
            if !v.get("level").is_some_and(Value::is_string) {
                self.err("E006", node, slot, "place needs level = \"<level name>\"".into());
            } else {
                self.check_indexed(Some(PickerType::Level), &v.get_string("level", ""), node, slot);
            }
            let ok = v.get("pos").is_some_and(|p| p.is_table() && p.arr().len() == 3 && p.arr().iter().all(Value::is_number));
            if !ok {
                self.err("E006", node, slot, "place needs pos = { x, y, z }".into());
            }
            if let Some(r) = v.get("radius")
                && !r.as_number().is_some_and(|n| n > 0.0)
            {
                self.err("E006", node, slot, "radius must be a positive number".into());
            }
            return true;
        }
        self.err("E006", node, slot, "place needs level+pos, restrictor, smart, ref or actor".into());
        false
    }

    fn check_duration(&mut self, v: &Value, node: Option<&str>, slot: &str) -> bool {
        // the same five forms the runtime resolves: a bare number is real seconds
        if let Some(n) = v.as_number() {
            if n < 0.0 {
                self.err("E006", node, slot, "duration must be a non-negative number of seconds".into());
                return false;
            }
            return true;
        }
        if !v.is_table() {
            self.err(
                "E006",
                node,
                slot,
                "expected a number of seconds or a duration { seconds = N } / { game_hours = N } / { game_minutes = N } / { game_seconds = N }".into(),
            );
            return false;
        }
        let mut alts = 0;
        for unit in ["seconds", "game_hours", "game_minutes", "game_seconds"] {
            if let Some(u) = v.get(unit) {
                alts += 1;
                if !u.as_number().is_some_and(|n| n >= 0.0) {
                    self.err("E006", node, slot, format!("{unit} must be a non-negative number"));
                }
            }
        }
        if alts != 1 {
            self.err("E006", node, slot, "duration needs exactly one of seconds / game_hours / game_minutes / game_seconds".into());
            return false;
        }
        true
    }

    /// One parameter against its catalog definition; `siblings` is the table it came out of.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn check_param_value(&mut self, k: &Kind, p: &Param, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, events_ok: bool, siblings: Option<&Value>) {
        let t = p.ty.as_str();
        let name = p.name.as_str();
        match t {
            "int" | "float" => {
                let Some(n) = v.as_number() else {
                    self.err("E006", node, slot, format!("'{name}' must be a number"));
                    return;
                };
                if t == "int" && !is_integer(n) {
                    self.err("E006", node, slot, format!("'{name}' must be an integer"));
                }
                if let Some(min) = p.min
                    && n < min
                {
                    self.err("E006", node, slot, format!("'{name}' must be >= {}", g(min)));
                }
                if let Some(max) = p.max
                    && n > max
                {
                    self.err("E006", node, slot, format!("'{name}' must be <= {}", g(max)));
                }
                return;
            }
            "bool" => {
                if !v.is_bool() {
                    self.err("E006", node, slot, format!("'{name}' must be true or false"));
                }
                return;
            }
            "enum" | "relation" => {
                let mut allowed: Vec<String> = p.enums.clone();
                if t == "relation" && allowed.is_empty() {
                    allowed = vec!["enemy".into(), "neutral".into(), "friend".into()];
                }
                let Some(s) = str_of(v) else {
                    self.err("E006", node, slot, format!("'{name}' must be one of the enum values"));
                    return;
                };
                if !allowed.is_empty() && !allowed.iter().any(|a| a == s) {
                    self.err("E006", node, slot, format!("'{name}' = \"{s}\" is not one of {}", allowed.join("|")));
                }
                return;
            }
            "text" => {
                let table_ok = v.as_table().is_some_and(|t| !t.fields.is_empty());
                if !v.is_string() && !table_ok {
                    self.err("E006", node, slot, format!("'{name}' must be a string or {{ rus = ..., eng = ... }}"));
                    return;
                }
                if let Some(tb) = v.as_table() {
                    for (key, x) in &tb.fields {
                        if !x.is_string() {
                            self.err("E006", node, slot, format!("'{name}'.{key} must be a string"));
                            return;
                        }
                    }
                }
                self.check_text(v, node, slot);
                return;
            }
            "value" => {
                if v.is_table() {
                    self.err("E006", node, slot, format!("'{name}' must be a bool, number or string"));
                } else if v.is_string() {
                    // a string written into a variable ends up in text through {var:name}
                    self.check_text(v, node, slot);
                }
                return;
            }
            "count_or_all" => {
                if v.as_str() == Some("all") {
                    return;
                }
                if !v.as_number().is_some_and(|n| is_integer(n) && n >= 1.0) {
                    self.err("E006", node, slot, format!("'{name}' must be a positive integer or \"all\""));
                }
                return;
            }
            "lua" => {
                let Some(code) = str_of(v) else {
                    self.err("E006", node, slot, format!("'{name}' must be a string with Lua code"));
                    return;
                };
                if let Err(e) = gz_luadata::syntax_check(code) {
                    if e.message.contains("cannot create") {
                        self.warn("W050", node, slot, format!("Lua syntax not checked: {}", e.message));
                    } else if e.line > 0 {
                        self.err("E050", node, slot, format!("Lua syntax error at {}:{}: {}", e.line, e.column, e.message));
                    } else {
                        self.err("E050", node, slot, format!("Lua syntax error: {}", e.message));
                    }
                }
                return;
            }
            "duration" => {
                self.check_duration(v, node, slot);
                return;
            }
            "place" => {
                self.check_place(v, node, slot, nidx, order);
                // "where the player stands" is a spawn point, not a destination
                if v.is_table() && v.has("actor") && matches!(k.id.as_str(), "objective.reach" | "actor_in_place" | "actor.teleport") {
                    self.err(
                        "E006",
                        node,
                        slot,
                        format!("a place relative to the player means nothing for {} - name a point, a restrictor, a smart or a ref", k.id),
                    );
                }
                return;
            }
            "npc_ref" => {
                self.check_npc_ref(v, node, slot, nidx, order, false, false);
                return;
            }
            "target_ref" => {
                self.check_npc_ref(v, node, slot, nidx, order, true, false);
                return;
            }
            "kill_target" => {
                self.check_npc_ref(v, node, slot, nidx, order, false, true);
                return;
            }
            "squad_ref" | "object_ref" => {
                self.check_object_ref(v, node, slot, nidx, order, t);
                // fetch's item and from expect a thing with an inventory or a lootable body
                if k.id == "objective.fetch" && t == "object_ref" {
                    self.note_item_use(v, nidx, order, slot);
                }
                return;
            }
            "spawn_spec" => {
                self.check_spawn_spec(v, node, slot, nidx, order, name);
                return;
            }
            // a kind that declares a row schema CREATES the items: its rows carry state
            "item_list" => {
                self.check_item_list(v, node, slot, nidx, order, !k.has_rows(), if k.has_rows() { Some(k) } else { None });
                return;
            }
            "loot_spawn" => {
                self.check_loot_spawn(v, node, slot, nidx, order);
                return;
            }
            "spawn_section" => {
                match v.as_str().filter(|s| !s.is_empty()) {
                    Some(s) => self.check_indexed(Some(PickerType::SpawnSection), s, node, slot),
                    None => self.err("E006", node, slot, format!("'{name}' must be a spawn section name")),
                }
                return;
            }
            "npc_list" => {
                self.check_npc_list(k, p, v, node, slot, nidx, order);
                return;
            }
            "destination" => {
                if !v.is_table() || !v.arr().is_empty() {
                    self.err("E006", node, slot, format!("'{name}' must be {{ path = }} / {{ ref = }} / {{ story = }} / {{ character = }} / {{ role = }} or a place"));
                    return;
                }
                if v.has("character") {
                    self.check_character_name(v.get("character"), node, slot);
                    return;
                }
                if let Some(path) = v.get("path") {
                    if !path.as_str().is_some_and(|s| !s.is_empty()) {
                        self.err("E006", node, slot, "path must be a waypoint path name".into());
                    }
                    return;
                }
                if let Some(role) = v.get("role") {
                    if !role.as_str().is_some_and(|s| !s.is_empty()) {
                        self.err("E006", node, slot, "role must be a role name".into());
                    }
                    return;
                }
                if let Some(story) = v.get("story") {
                    if !story.as_str().is_some_and(|s| !s.is_empty()) {
                        self.err("E006", node, slot, "story must be a story id".into());
                    }
                    return;
                }
                self.check_place(v, node, slot, nidx, order);
                return;
            }
            "role" => {
                if let Some(s) = v.as_str() {
                    if s.is_empty() {
                        self.err("E006", node, slot, format!("'{name}' names no role"));
                    }
                    return;
                }
                if !v.is_table() || v.arr().is_empty() {
                    self.err("E006", node, slot, format!("'{name}' must be a role name or a list of them"));
                    return;
                }
                for x in v.arr() {
                    if !x.as_str().is_some_and(|s| !s.is_empty()) {
                        self.err("E006", node, slot, format!("'{name}' list must hold role names"));
                    }
                }
                return;
            }
            "role_or_actor" | "attack_target" => {
                let attack = t == "attack_target";
                if !v.is_table() || !v.arr().is_empty() {
                    self.err(
                        "E006",
                        node,
                        slot,
                        format!(
                            "'{name}' must be {{ actor = true }} / {{ role = ... }}{}",
                            if attack { " / { community = ... } / { story = ... }" } else { " / { ref = ... } / { story = ... }" }
                        ),
                    );
                    return;
                }
                let mut forms = 0;
                if let Some(a) = v.get("actor") {
                    forms += 1;
                    if !a.is_bool() {
                        self.err("E006", node, slot, "actor must be true".into());
                    }
                }
                if let Some(r) = v.get("role") {
                    forms += 1;
                    if !r.as_str().is_some_and(|s| !s.is_empty()) {
                        self.err("E006", node, slot, "role must be a role name".into());
                    }
                }
                if attack {
                    if let Some(cm) = v.get("community") {
                        forms += 1;
                        if !cm.is_string() {
                            self.err("E006", node, slot, "community must be a string".into());
                        }
                    }
                    if v.has("story") || v.has("ref") || v.has("character") || v.has("profile") {
                        forms += 1;
                        self.check_npc_ref(v, node, slot, nidx, order, true, false);
                    }
                } else if v.has("story") || v.has("ref") || v.has("character") {
                    // something to walk after: a ref the caller spawned, an object, a character
                    forms += 1;
                    if v.has("ref") {
                        self.note_ref_use(v, nidx, order, slot);
                    }
                    if v.has("character") {
                        self.check_character_name(v.get("character"), node, slot);
                    }
                }
                if forms == 0 {
                    self.err("E006", node, slot, format!("'{name}' names nobody"));
                } else if forms > 1 {
                    self.err("E006", node, slot, format!("'{name}' must name exactly one target"));
                }
                return;
            }
            "role_bindings" => {
                if !v.is_table() || !v.arr().is_empty() {
                    self.err("E006", node, slot, format!("'{name}' must be a table {{ role = {{ story = ... }}, ... }}"));
                    return;
                }
                for (key, b) in v.fields() {
                    let sl = format!("{slot}/{key}");
                    if b.is_table() && !b.arr().is_empty() {
                        for x in b.arr() {
                            self.check_npc_ref(x, node, &sl, nidx, order, true, true);
                        }
                    } else {
                        self.check_npc_ref(b, node, &sl, nidx, order, true, true);
                    }
                }
                return;
            }
            "cases_cond" => {
                self.check_cases(v, node, slot, nidx, order, false, events_ok);
                return;
            }
            "cases_weight" => {
                self.check_cases(v, node, slot, nidx, order, true, events_ok);
                return;
            }
            "cond_list" => {
                if !v.is_table() || v.arr().is_empty() {
                    self.err("E006", node, slot, format!("'{name}' must be a non-empty list of conditions"));
                    return;
                }
                for (i, c) in v.arr().iter().enumerate() {
                    let sl = format!("{slot}/of:{i}");
                    self.check_cond_value(c, node, &sl, nidx, order, events_ok);
                }
                return;
            }
            _ => {}
        }
        // everything else is a plain string: identifiers, sections, names
        let Some(s) = str_of(v) else {
            self.err("E006", node, slot, format!("'{name}' must be a string"));
            return;
        };
        match t {
            "global_name" => {
                if s.is_empty() {
                    self.err("E006", node, slot, format!("'{name}' names no global variable"));
                    return;
                }
                if s.contains('.') || self.ctx.file_path.is_none() {
                    return;
                }
                if !self.ctx.globals.iter().any(|gv| gv == s) {
                    self.err("E030", node, slot, format!("global variable '{s}' is not declared in mod.ltx [vars] - add it under Global in the Variables"));
                }
            }
            "ref_name" => match k.id.as_str() {
                "spawn.squad" => self.note_ref_decl(s, nidx, order, slot, "squad"),
                "spawn.object" => self.note_ref_decl(s, nidx, order, slot, "object"),
                "item.spawn" => self.note_ref_decl(s, nidx, order, slot, "item"),
                _ => {}
            },
            "objective_id" => {
                // only meaningful next to its task: `task` for the task.* actions, `spot_task`
                // for the objective nodes
                let owner = siblings.and_then(|sb| sb.get("task").or_else(|| sb.get("spot_task")));
                let task = owner.and_then(Value::as_str).and_then(|o| self.q.find_task(o));
                match task {
                    None => self.err("E030", node, slot, format!("objective '{s}' names no task")),
                    Some(task) if task.objective(s).is_none() => {
                        let tid = task.id.clone();
                        self.err("E030", node, slot, format!("task '{tid}' declares no objective '{s}'"));
                    }
                    Some(_) => {}
                }
            }
            "task_id" => {
                if self.q.find_task(s).is_none() {
                    self.err("E030", node, slot, format!("task '{s}' is not declared in tasks"));
                }
            }
            "var_name" => {
                if self.q.find_var(s).is_none() {
                    self.err("E030", node, slot, format!("variable '{s}' is not declared in vars"));
                }
            }
            "node_id" => {
                if self.q.find_node(s).is_none() {
                    self.err("E030", node, slot, format!("node '{s}' does not exist"));
                }
            }
            "behaviour_id" => {
                if !s.contains('.')
                    && let Some(ids) = self.ctx.project_behaviour_ids
                    && !ids.iter().any(|i| i == s)
                {
                    self.err("E030", node, slot, format!("behaviour graph '{s}' is not a graph of this project"));
                }
                if self.q.is_behaviour() && s == self.q.id {
                    self.err("E113", node, slot, "a behaviour graph may not run itself".into());
                }
            }
            "quest_id" => {
                if !s.contains('.')
                    && let Some(ids) = self.ctx.project_quest_ids
                    && s != self.q.id
                    && !ids.iter().any(|i| i == s)
                {
                    self.err("E030", node, slot, format!("quest '{s}' is not a quest of this project"));
                }
            }
            _ => self.check_indexed(PickerType::from_name(t), s, node, slot),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_npc_list(&mut self, k: &Kind, p: &Param, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32) {
        const ROW_KEYS: [&str; 6] = ["character", "section", "ref", "unique", "dead", "yaw"];
        if !v.is_table() || v.arr().is_empty() || !v.fields().is_empty() {
            self.err("E006", node, slot, format!("'{}' must be a non-empty list of rows", p.name));
            return;
        }
        for (i, row) in v.arr().iter().enumerate() {
            let sl = format!("{slot}/row:{}", i + 1);
            if !row.is_table() || !row.arr().is_empty() {
                self.err("E006", node, &sl, "a row must be a table { character = ... | section = ..., ref = ..., unique = ... }".into());
                continue;
            }
            for (key, _) in row.fields() {
                let known = if k.has_rows() { k.row_param(key).is_some() } else { ROW_KEYS.contains(&key.as_str()) };
                if !known {
                    self.err("E006", node, &sl, format!("unknown key '{key}' in a row"));
                }
            }
            let ch = row.get("character");
            let se = row.get("section");
            let has_ch = ch.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            let has_se = se.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            if has_ch == has_se {
                self.err("E022", node, &sl, "a row names a character OR a section - exactly one".into());
            }
            if has_ch {
                self.check_character_name(ch, node, &sl);
            }
            if has_se && let Some(s) = se.and_then(Value::as_str) {
                self.check_indexed(Some(PickerType::SpawnSection), s, node, &sl);
            }
            if let Some(r) = row.get("ref") {
                match r.as_str() {
                    Some(s) if valid_id(s) => self.note_ref_decl(s, nidx, order, &sl, "object"),
                    _ => self.err("E006", node, &sl, "ref must be a name [a-z0-9_]+".into()),
                }
            }
            if row.get("unique").is_some_and(|u| !u.is_bool()) {
                self.err("E006", node, &sl, "unique must be true or false".into());
            }
            if row.get("dead").is_some_and(|d| !d.is_bool()) {
                self.err("E006", node, &sl, "dead must be true or false".into());
            }
            if row.get("yaw").is_some_and(|y| !y.is_number()) {
                self.err("E006", node, &sl, "yaw must be a number".into());
            }
        }
    }

    /// Every parameter of an entity against its kind (`CheckParams`).
    #[allow(clippy::too_many_arguments)]
    fn check_params(&mut self, k: &Kind, params: &Value, node: Option<&str>, prefix: &str, nidx: Option<usize>, order: i32, events_ok: bool) {
        if !params.is_nil() && !params.is_table() {
            self.err("E006", node, prefix, "params must be a table".into());
            return;
        }
        for p in &k.params {
            let slot = slot_name(prefix, &p.name);
            match params.get(&p.name) {
                None | Some(Value::Nil) => {
                    if p.required {
                        self.err("E006", node, &slot, format!("required parameter '{}' ({}) is missing", p.name, p.ty));
                    }
                }
                Some(v) => self.check_param_value(k, p, v, node, &slot, nidx, order, events_ok, Some(params)),
            }
        }
    }

    /// A kind belongs to a quest, to a behaviour graph, or to both (`CheckDoc`).
    fn check_doc(&mut self, k: &Kind, node: Option<&str>, slot: &str) -> bool {
        let behaviour = self.q.is_behaviour();
        if k.in_doc(behaviour) {
            return true;
        }
        self.err("E023", node, slot, format!("'{}' cannot be used in a {}", k.id, if behaviour { "behaviour graph" } else { "quest" }));
        false
    }

    /// `who` names roles the graph declares; so does `{ role = }` in a place or a target
    /// (`CheckWho`).
    fn check_who(&mut self, k: &Kind, params: &Value, node: Option<&str>, slot: &str) {
        let mut names: Vec<String> = Vec::new();
        if k.param("who").is_some()
            && let Some(who) = params.get("who")
        {
            match who {
                Value::String(s) => names.push(s.to_string()),
                Value::Table(t) => names.extend(t.arr.iter().filter_map(Value::as_str).map(str::to_owned)),
                _ => {}
            }
        }
        for holder in ["place", "target", "area"] {
            if let Some(h) = params.get(holder)
                && h.is_table()
                && let Some(Value::String(role)) = h.get("role")
                && !role.is_empty()
            {
                if self.q.is_behaviour() {
                    names.push(role.to_string());
                } else {
                    self.err("E024", node, slot, format!("a quest has no roles - name the NPC by {{ ref = }} or {{ story = }} instead of {{ role = \"{role}\" }}"));
                }
            }
        }
        for n in names {
            if !self.q.roles.iter().any(|r| r.name == n) {
                self.err("E024", node, slot, format!("no role called '{n}' is declared by this behaviour graph"));
            }
        }
    }

    fn check_cond(&mut self, cd: &Cond, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, events_ok: bool) {
        let cat = self.cat;
        let Some(k) = cat.find(&cd.kind).filter(|k| k.is(USE_COND)) else {
            if self.catalog {
                self.err("E005", node, slot, format!("'{}' is not a condition kind", cd.kind));
            }
            return;
        };
        if !self.check_doc(k, node, slot) {
            return;
        }
        if k.event && !events_ok {
            self.err("E020", node, slot, format!("event condition '{}' is only allowed in trigger.when / wait.when / wait.any", cd.kind));
        }
        self.check_params(k, &cd.params, node, slot, nidx, order, events_ok);
        self.check_who(k, &cd.params, node, slot);
    }

    fn check_cond_value(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, events_ok: bool) {
        match cond_from_value(v) {
            Ok(cd) => self.check_cond(&cd, node, slot, nidx, order, events_ok),
            Err(e) => self.err("E006", node, slot, e),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_cases(&mut self, v: &Value, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32, weighted: bool, events_ok: bool) {
        if !v.is_table() || v.arr().is_empty() {
            self.err(
                "E021",
                node,
                slot,
                format!("cases must be a non-empty list {{ {{ name = ..., {} }}, ... }}", if weighted { "weight = N" } else { "cond = { ... }" }),
            );
            return;
        }
        let mut names: Vec<String> = Vec::new();
        for (i, cs) in v.arr().iter().enumerate() {
            let sl = format!("{slot}{}case:{i}", if slot.is_empty() { "" } else { "/" });
            if !cs.is_table() {
                self.err("E021", node, &sl, "case must be a table".into());
                continue;
            }
            let name = cs.get_string("name", "");
            if !valid_id(&name) {
                self.err("E021", node, &sl, format!("case name '{name}' must be [a-z0-9_]+"));
            }
            if names.contains(&name) {
                self.err("E021", node, &sl, format!("duplicate case name '{name}'"));
            }
            names.push(name.clone());
            if weighted {
                if !cs.get("weight").and_then(Value::as_number).is_some_and(|w| w > 0.0) {
                    self.err("E021", node, &sl, format!("case '{name}' needs weight > 0"));
                }
            } else {
                let Some(cl) = cs.get("cond").filter(|c| c.is_table()) else {
                    self.err("E021", node, &sl, format!("case '{name}' needs cond = {{ {{ kind = ... }}, ... }}"));
                    continue;
                };
                if cl.arr().is_empty() && cl.has("kind") {
                    self.check_cond_value(cl, node, &sl, nidx, order, events_ok);
                }
                for (j, c) in cl.arr().iter().enumerate() {
                    let sl2 = format!("{sl}/cond:{j}");
                    self.check_cond_value(c, node, &sl2, nidx, order, events_ok);
                }
            }
        }
    }

    fn check_action(&mut self, a: &Action, node: Option<&str>, slot: &str, nidx: Option<usize>, order: i32) {
        let cat = self.cat;
        let Some(k) = cat.find(&a.kind).filter(|k| k.is(USE_EXTRA)) else {
            if self.catalog {
                self.err("E005", node, slot, format!("'{}' is not an extra-action kind", a.kind));
            }
            return;
        };
        if !self.check_doc(k, node, slot) {
            return;
        }
        self.check_params(k, &a.params, node, slot, nidx, order, false);
        self.check_who(k, &a.params, node, slot);
        // an action kind with exclusive forms (item.spawn) is checked like a node kind
        self.check_kind_form(&k.id, &a.params, node, slot);
        if k.id == "spawn.squad" && a.params.get("hold").and_then(Value::as_bool) == Some(false) {
            self.warn("W031", node, slot, "spawn.squad without hold - the squad may leave for the simulation".into());
        }
        // the one-line spelling IS one row; beside an items list the state belongs to no row
        if k.id == "item.spawn" && a.params.is_table() {
            let listed = a.params.has("items");
            for rp in &k.rows {
                if rp.name == "section" || rp.name == "count" || !a.params.has(&rp.name) {
                    continue;
                }
                if listed {
                    self.err("E022", node, slot, format!("item.spawn: the state belongs to the rows now - move '{}' into the row it applies to", rp.name));
                }
            }
            let section = a.params.get_string("section", "");
            if !listed && !section.is_empty() {
                let mut row = Value::table();
                for (key, val) in a.params.fields() {
                    if k.row_param(key).is_some() {
                        row.set(key, val.clone());
                    }
                }
                self.check_row_state(&row, k, &section, node, slot, nidx, order);
            }
        }
        if k.id == "actor.teleport"
            && let Some(level) = self.ctx.level.filter(|l| !l.is_empty())
            && let Some(place) = a.params.get("place")
            && place.is_table()
            && place.has("level")
        {
            let lv = place.get_string("level", "");
            if !lv.is_empty() && !lv.eq_ignore_ascii_case(level) {
                self.err("E031", node, slot, format!("actor.teleport to level '{lv}' - the actor is on '{level}'; there is no cross-level teleport"));
            }
        }
    }

    // ---- graph rules ---------------------------------------------------------------------------

    fn check_dialogs(&mut self) {
        let q = self.q;
        let idx = q.index();
        let mut in_chain = vec![false; q.nodes.len()];
        for topic in q.nodes.iter().filter(|n| text::is_topic_kind(&n.kind)) {
            let actor_first = topic.params.get_string("initiator", "actor") != "npc";
            let hops = q.dialog_chain(&topic.id);
            let mut parity = vec![0u8; q.nodes.len()];
            for &(ni, depth) in &hops {
                if let Some(c) = in_chain.get_mut(ni) {
                    *c = true;
                }
                if depth == 0 {
                    continue;
                }
                if let Some(pp) = parity.get_mut(ni) {
                    *pp |= if depth & 1 == 1 { 2 } else { 1 };
                }
            }
            for (ni, &par) in parity.iter().enumerate() {
                if par == 0 {
                    continue;
                }
                let Some(ph) = q.nodes.get(ni) else { continue };
                let is_actor = ph.kind == "dialog.actor_phrase";
                // the initiator's side speaks at even depth
                let want: u8 = if is_actor == actor_first { 1 } else { 2 };
                if par & !want != 0 {
                    self.err(
                        "E010",
                        Some(&ph.id),
                        "",
                        format!(
                            "{} at the wrong depth in dialog '{}' - speakers must alternate ({} speaks at {} depth here)",
                            if is_actor { "actor phrase" } else { "npc phrase" },
                            topic.id,
                            if is_actor { "the actor" } else { "the npc" },
                            if want == 1 { "even" } else { "odd" }
                        ),
                    );
                }
            }
            let has_phrase = topic
                .targets("next")
                .is_some_and(|nx| nx.iter().any(|t| idx.get(t.as_str()).and_then(|&j| q.nodes.get(j)).is_some_and(|n| text::is_phrase_kind(&n.kind))));
            if !has_phrase {
                self.warn("W012", Some(&topic.id), "", "dialog topic has no phrase after it (out.next)".into());
            }
            if topic.once_set && !topic.once && topic.cond.is_empty() {
                self.warn("W013", Some(&topic.id), "", "topic with once=false and no cond will be offered forever".into());
            }
        }
        // W011: a non-leaf PHRASE needs an unconditional continuation
        for n in q.nodes.iter().filter(|n| text::is_phrase_kind(&n.kind)) {
            let Some(nx) = n.targets("next") else { continue };
            let (mut phrases, mut unconditional, mut to_actor) = (0, 0, false);
            for t in nx {
                let Some(tn) = idx.get(t.as_str()).and_then(|&j| q.nodes.get(j)) else { continue };
                if !text::is_phrase_kind(&tn.kind) {
                    continue;
                }
                phrases += 1;
                if tn.kind == "dialog.actor_phrase" {
                    to_actor = true;
                }
                // the runtime counts `once` as a precondition too
                if tn.cond.is_empty() && !(tn.once_set && tn.once) {
                    unconditional += 1;
                }
            }
            if phrases > 0 && (if to_actor { unconditional < phrases } else { unconditional == 0 }) {
                self.warn(
                    "W011",
                    Some(&n.id),
                    "out:next",
                    if to_actor {
                        "conditional player options - the game adds a hidden way out that does not fire the topic's done pin".into()
                    } else {
                        "no unconditional continuation - the game adds an automatic reply when every phrase is filtered".into()
                    },
                );
            }
        }
        // E010 (unreachable from a topic) / E011 (entered from the world)
        for (i, n) in q.nodes.iter().enumerate() {
            if !text::is_phrase_kind(&n.kind) {
                continue;
            }
            if !in_chain.get(i).copied().unwrap_or(false) {
                self.err("E010", Some(&n.id), "", "phrase is not reachable from any dialog.topic through next pins".into());
            }
            for src in &q.nodes {
                for p in &src.out {
                    for t in &p.targets {
                        if *t != n.id {
                            continue;
                        }
                        if !text::is_dialog_kind(&src.kind) {
                            self.err(
                                "E011",
                                Some(&n.id),
                                "",
                                format!(
                                    "phrase is entered from '{}' ({}.{}) - phrases may only follow a topic or another phrase",
                                    src.id, src.kind, p.name
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    /// `W030`: a ref a path from a trigger reaches without its declaring node; `E030` when
    /// nothing declares it (`CheckRefs`).
    fn check_refs(&mut self) {
        let q = self.q;
        let idx = q.index();
        let item_uses = std::mem::take(&mut self.item_uses);
        for u in &item_uses {
            if self.decls.iter().any(|d| d.reference == u.reference && d.kind == "squad") {
                let node = self.node_id(u.node);
                self.warn("W031", node, &u.slot, format!("ref '{}' was created by spawn.squad - it is a squad, not an item or a container", u.reference));
            }
        }
        let uses = std::mem::take(&mut self.uses);
        let triggers: Vec<usize> = q.nodes.iter().enumerate().filter(|(_, n)| self.cat.is_trigger(&n.kind)).map(|(i, _)| i).collect();
        for u in &uses {
            let node = self.node_id(u.node);
            let mut declaring: Vec<Option<usize>> = Vec::new();
            let mut declared_here_before = false;
            for d in self.decls.iter().filter(|d| d.reference == u.reference) {
                declaring.push(d.node);
                if d.node == u.node && d.order <= u.order {
                    declared_here_before = true;
                }
            }
            // a behaviour graph sees the refs of whoever runs it: taken on trust
            if declaring.is_empty() && q.is_behaviour() {
                continue;
            }
            if declaring.is_empty() {
                self.err("E030", node, &u.slot, format!("ref '{}' is never created by this quest", u.reference));
                continue;
            }
            let Some(use_node) = u.node else { continue };
            if declared_here_before {
                continue;
            }
            // reachability from the triggers avoiding the declaring nodes
            let mut seen = vec![false; q.nodes.len()];
            let mut stack: Vec<usize> = Vec::new();
            for &t in &triggers {
                if let Some(s) = seen.get_mut(t) {
                    *s = true;
                }
                stack.push(t);
            }
            let mut reached = false;
            while let Some(cur) = stack.pop() {
                if cur == use_node {
                    reached = true;
                    break;
                }
                if declaring.contains(&Some(cur)) {
                    continue;
                }
                let Some(n) = q.nodes.get(cur) else { continue };
                for p in &n.out {
                    for t in &p.targets {
                        if let Some(&j) = idx.get(t.as_str())
                            && let Some(s) = seen.get_mut(j)
                            && !*s
                        {
                            *s = true;
                            stack.push(j);
                        }
                    }
                }
            }
            if reached {
                self.warn("W030", node, &u.slot, format!("ref '{}' may be used before the node that creates it (not on every path)", u.reference));
            }
        }
    }

    fn check_node(&mut self, i: usize, n: &Node, triggers: &mut usize, ends: &mut usize) {
        let q = self.q;
        let cat = self.cat;
        let nid = Some(n.id.as_str());
        if !valid_id(&n.id) {
            self.err("E004", nid, "", format!("node id '{}' must be [a-z0-9_]+", n.id));
        }
        if q.nodes.iter().take(i).any(|m| m.id == n.id) {
            self.err("E004", nid, "", format!("duplicate node id '{}'", n.id));
        }
        let k = cat.find(&n.kind);
        if cat.is_trigger(&n.kind) {
            *triggers += 1;
        }
        if n.kind == "flow.end" {
            *ends += 1;
        }
        match k.filter(|k| k.is(USE_MAIN) || k.is(USE_TRIGGER)) {
            None => {
                if self.catalog {
                    self.err("E005", nid, "", format!("'{}' is not a node kind (main action or trigger)", n.kind));
                }
            }
            Some(k) => {
                self.check_doc(k, nid, "");
                // event conditions are only for the edge-triggered kinds
                let events_ok = matches!(n.kind.as_str(), "trigger.when" | "wait.when" | "wait.any");
                self.check_params(k, &n.params, nid, "", Some(i), 200, events_ok);
                self.check_who(k, &n.params, nid, "");
                self.check_kind_form(&n.kind, &n.params, nid, "");
                let idx = q.index();
                let case_names: Vec<String> = if k.pins_from_cases {
                    n.params.get("cases").map(|c| c.arr().iter().map(|x| x.get_string("name", "")).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
                } else {
                    Vec::new()
                };
                for p in &n.out {
                    let sl = format!("out:{}", p.name);
                    if !k.has_pin(&p.name) && !case_names.iter().any(|c| c == p.name.as_str()) {
                        self.err("E007", nid, &sl, format!("pin '{}' is not declared by {}", p.name, n.kind));
                    }
                    let mut seen: FxHashSet<&str> = FxHashSet::default();
                    for target in &p.targets {
                        if !seen.insert(target.as_str()) {
                            self.warn("W022", nid, &sl, format!("target '{target}' is listed twice, the duplicate edge is ignored"));
                            continue;
                        }
                        let Some(tn) = idx.get(target.as_str()).and_then(|&j| q.nodes.get(j)) else {
                            self.err("E007", nid, &sl, format!("target node '{target}' does not exist"));
                            continue;
                        };
                        if cat.is_trigger(&tn.kind) {
                            self.err("E007", nid, &sl, format!("edge leads into trigger '{}' - triggers have no input", tn.id));
                        }
                    }
                }
                if n.kind == "flow.join" && q.in_degree(&n.id) == 0 {
                    self.err("E009", nid, "", "flow.join has no incoming edges".into());
                }
                if k.wait && n.out_degree() == 0 && !text::is_topic_kind(&n.kind) {
                    self.warn("W021", nid, "", "waiting node has no outgoing edges - the quest will stop here".into());
                }
                for (ci, c) in n.cond.iter().enumerate() {
                    let sl = format!("cond:{ci}");
                    self.check_cond(c, nid, &sl, Some(i), ci as i32, events_ok);
                }
            }
        }
        for (ai, a) in n.on_enter.iter().enumerate() {
            let sl = format!("enter:{ai}");
            self.check_action(a, nid, &sl, Some(i), 100 + ai as i32);
        }
        for (ai, a) in n.on_exit.iter().enumerate() {
            let sl = format!("exit:{ai}");
            self.check_action(a, nid, &sl, Some(i), 300 + ai as i32);
        }
        if !n.comment.is_empty() {
            self.check_text(&Value::from(n.comment.as_str()), nid, "comment");
        }
    }

    fn check_header(&mut self) {
        let q = self.q;
        let ctx = self.ctx;
        let vkey = if q.is_behaviour() { "beh" } else { "nq" };
        if q.nq != 1 {
            if q.nq == 0 {
                self.err("E001", None, "", format!("{vkey} (format version) is missing - write {vkey} = 1"));
            } else {
                self.err("E001", None, "", format!("{vkey} = {} is not supported (this editor writes {vkey} = 1)", q.nq));
            }
        }
        // the extension is the kind of document and the header key has to agree
        if let Some(fp) = ctx.file_path.filter(|f| !f.is_empty()) {
            if q.is_behaviour() && is_quest_path(fp) {
                self.err("E001", None, "", "a .nqasset is a quest, but this document says beh = - rename it to .behasset or write nq =".into());
            } else if !q.is_behaviour() && is_behaviour_path(fp) {
                self.err("E001", None, "", "a .behasset is a behaviour graph, but this document says nq = - rename it to .nqasset or write beh =".into());
            }
        }
        if !valid_id(&q.id) {
            self.err("E002", None, "", format!("quest id '{}' must be [a-z0-9_]+", q.id));
        } else if let Some(ids) = ctx.project_quest_ids
            && ids.contains(&q.id)
        {
            self.err("E002", None, "", format!("quest id '{}' is used by another quest of this project", q.id));
        }
        let behaviour = q.is_behaviour();
        if behaviour {
            if !q.tasks.is_empty() {
                self.err("E006", None, "param:tasks", "a behaviour graph has no tasks of its own - those belong to the quest that runs it".into());
            }
            if q.roles.is_empty() {
                self.err("E006", None, "param:roles", "a behaviour graph needs at least one role - it has nobody to drive otherwise".into());
            }
            for (i, r) in q.roles.iter().enumerate() {
                if !valid_id(&r.name) {
                    self.err("E006", None, "param:roles", format!("role name '{}' must be [a-z0-9_]+", r.name));
                }
                for other in q.roles.iter().skip(i + 1) {
                    if other.name == r.name {
                        self.err("E006", None, "param:roles", format!("role '{}' is declared twice", r.name));
                    }
                }
                if !r.title.is_nil() {
                    self.check_text(&r.title, None, "param:roles");
                }
                if !r.default.is_nil() && !r.default.is_table() {
                    self.err("E006", None, "param:roles", format!("the default of role '{}' must be a reference table {{ story = ... }}", r.name));
                }
            }
        } else if !q.roles.is_empty() {
            self.err("E006", None, "param:roles", "roles belong to a behaviour graph (.behasset) - a quest has none".into());
        }
        if !behaviour && q.activation != "auto" && q.activation != "manual" {
            self.err("E006", None, "param:activation", "activation must be \"auto\" or \"manual\"".into());
        }
        // an absent title is legal - the runtime falls back to the id
        if !q.title.is_nil() {
            self.check_text(&q.title, None, "param:title");
            if !q.title.is_string() && !q.title.as_table().is_some_and(|t| !t.fields.is_empty()) {
                self.err("E006", None, "param:title", "title must be a string or { rus = ..., eng = ... }".into());
            }
        }
        for v in &q.vars {
            if !valid_id(&v.name) {
                self.err("E006", None, "param:vars", format!("variable name '{}' must be [a-z0-9_]+", v.name));
            }
            if v.value.is_table() || v.value.is_nil() {
                self.err("E006", None, "param:vars", format!("variable '{}' default must be a bool, number or string", v.name));
            }
            if v.value.is_string() {
                let slot = format!("param:vars/{}", v.name);
                self.check_text(&v.value, None, &slot);
            }
        }
        for t in &q.tasks {
            let sl = format!("task:{}", t.id);
            if !valid_id(&t.id) {
                self.err("E006", None, &sl, format!("task id '{}' must be [a-z0-9_]+", t.id));
            }
            if !t.title.is_nil() {
                self.check_text(&t.title, None, &sl);
            }
            if !t.descr.is_nil() {
                self.check_text(&t.descr, None, &sl);
            }
            if t.ty != "additional" && t.ty != "storyline" {
                self.err("E006", None, &sl, "task type must be \"additional\" or \"storyline\"".into());
            }
            if !t.target.is_nil() {
                self.check_npc_ref(&t.target, None, &sl, None, 0, true, false);
            }
            for (j, o) in t.objectives.iter().enumerate() {
                let osl = format!("task:{}/objective:{}", t.id, o.id);
                if !valid_id(&o.id) {
                    self.err("E006", None, &osl, format!("objective id '{}' must be [a-z0-9_]+", o.id));
                }
                if t.objectives.iter().take(j).any(|x| x.id == o.id) {
                    self.err("E006", None, &osl, format!("duplicate objective id '{}'", o.id));
                }
                if !o.title.is_nil() {
                    self.check_text(&o.title, None, &osl);
                }
                if !o.descr.is_nil() {
                    self.check_text(&o.descr, None, &osl);
                }
                if !o.target.is_nil() {
                    self.check_npc_ref(&o.target, None, &osl, None, 0, true, false);
                }
            }
        }
        if let Some(fp) = ctx.file_path.filter(|f| !f.is_empty()) {
            let base = crate::asset::base_name(fp);
            // by the rule a new quest is named with: "NewQuest.nqasset" IS newquest
            if !q.id.is_empty() && id_from_name(&base) != q.id {
                self.warn("W061", None, "", format!("file name '{base}' differs from quest id '{}'", q.id));
            }
        }
    }
}

/// The full rule set (`NqValidate::Run`).
#[must_use]
pub fn run(q: &Graph, catalog: &Catalog, ctx: &Context<'_>) -> Vec<NqProblem> {
    let mut v = V {
        q,
        cat: catalog,
        ctx,
        out: Vec::new(),
        catalog: catalog.loaded(),
        pickers: ctx.pickers.filter(|p| p.available()),
        decls: Vec::new(),
        item_uses: Vec::new(),
        uses: Vec::new(),
    };
    v.check_header();
    let (mut triggers, mut ends) = (0, 0);
    for (i, n) in q.nodes.iter().enumerate() {
        v.check_node(i, n, &mut triggers, &mut ends);
    }
    if triggers == 0 {
        v.err("E008", None, "", "the quest has no trigger node (trigger.start / trigger.when)".into());
    }
    if ends == 0 {
        v.warn("W070", None, "", "no flow.end - the quest completes implicitly when nothing is left to do".into());
    }
    let reach = q.reachable(catalog, None);
    for (i, n) in q.nodes.iter().enumerate() {
        if !reach.get(i).copied().unwrap_or(false) && !catalog.is_trigger(&n.kind) {
            v.warn("W020", Some(&n.id), "", "node is not reachable from any trigger".into());
        }
    }
    v.check_dialogs();
    v.check_refs();
    v.out
}

/// The refs the graph declares and what made each (`squad`, `object`, `item`), first of a name
/// only (`DeclaredRefsEx`).
#[must_use]
pub fn declared_refs(q: &Graph, catalog: &Catalog) -> Vec<(String, &'static str)> {
    let ctx = Context::default();
    let mut v = V { q, cat: catalog, ctx: &ctx, out: Vec::new(), catalog: catalog.loaded(), pickers: None, decls: Vec::new(), item_uses: Vec::new(), uses: Vec::new() };
    for (i, n) in q.nodes.iter().enumerate() {
        let nid = Some(n.id.as_str());
        if let Some(k) = catalog.find(&n.kind) {
            v.check_params(k, &n.params, nid, "", Some(i), 200, true);
        }
        for (a, act) in n.on_enter.iter().enumerate() {
            if let Some(k) = catalog.find(&act.kind) {
                v.check_params(k, &act.params, nid, "", Some(i), 100 + a as i32, false);
            }
        }
        for (a, act) in n.on_exit.iter().enumerate() {
            if let Some(k) = catalog.find(&act.kind) {
                v.check_params(k, &act.params, nid, "", Some(i), 300 + a as i32, false);
            }
        }
    }
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for d in v.decls {
        if !out.iter().any(|(r, _)| *r == d.reference) {
            out.push((d.reference, d.kind));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::parse;

    fn bundled() -> Catalog {
        let bytes = include_bytes!("../../../gamedata/configs/nq/catalog.ltx");
        let text = gz_base::text::cp1251_to_utf8(bytes).into_owned();
        Catalog::assemble(&[crate::catalog::SourceFile { label: "bundled".into(), text }], false, None, 1)
    }

    #[test]
    fn the_examples_validate_clean() {
        let cat = bundled();
        for (name, text) in [
            ("dialog_branching", include_str!("../../../docs/nq/examples/dialog_branching.nqasset")),
            ("linear_fetch", include_str!("../../../docs/nq/examples/linear_fetch.nqasset")),
            ("parallel_triggers", include_str!("../../../docs/nq/examples/parallel_triggers.nqasset")),
        ] {
            let g = match parse(&cat, text, name) {
                Ok(g) => g,
                Err(e) => panic!("{name}: {}", e.message),
            };
            let path = format!("quests/{name}.nqasset");
            let ids: Vec<String> = Vec::new();
            let ctx = Context { file_path: Some(&path), project_quest_ids: Some(&ids), project_behaviour_ids: Some(&ids), ..Context::default() };
            let problems = run(&g, &cat, &ctx);
            let errors: Vec<String> = problems.iter().filter(|p| p.is_error()).map(NqProblem::text).collect();
            assert!(errors.is_empty(), "{name}: {errors:?}");
        }
    }

    #[test]
    fn codes_and_messages() {
        let cat = bundled();
        let text = r#"return { nq = 1, id = "Bad", title = "x", nodes = {
            { id = "a", kind = "flow.step", out = { next = { "ghost", "ghost" }, zap = "b" } },
            { id = "b", kind = "dialog.npc_phrase", params = { text = "hi" } },
            { id = "a", kind = "nope.kind" },
        } }"#;
        let g = match parse(&cat, text, "bad") {
            Ok(g) => g,
            Err(e) => panic!("{}", e.message),
        };
        let p = run(&g, &cat, &Context::default());
        let texts: Vec<String> = p.iter().map(NqProblem::text).collect();
        let has = |s: &str| texts.iter().any(|t| t == s);
        assert!(has("E002: quest id 'Bad' must be [a-z0-9_]+"), "{texts:?}");
        assert!(has("E007 a[out:zap]: pin 'zap' is not declared by flow.step"), "{texts:?}");
        assert!(has("E007 a[out:next]: target node 'ghost' does not exist"), "{texts:?}");
        assert!(has("W022 a[out:next]: target 'ghost' is listed twice, the duplicate edge is ignored"), "{texts:?}");
        assert!(has("E004 a: duplicate node id 'a'"), "{texts:?}");
        assert!(has("E005 a: 'nope.kind' is not a node kind (main action or trigger)"), "{texts:?}");
        assert!(has("E008: the quest has no trigger node (trigger.start / trigger.when)"), "{texts:?}");
        assert!(has("E011 b: phrase is entered from 'a' (flow.step.zap) - phrases may only follow a topic or another phrase"), "{texts:?}");
        assert!(has("W070: no flow.end - the quest completes implicitly when nothing is left to do"), "{texts:?}");
        assert_eq!(to_text(&[]), "problems: none");
    }

    #[test]
    fn refs_and_behaviour_rules() {
        let cat = bundled();
        let text = r#"return { nq = 1, id = "q", nodes = {
            { id = "s", kind = "trigger.start", out = { next = "k" } },
            { id = "k", kind = "npc.kill", params = { npc = { ref = "nobody" } }, out = { next = "e" } },
            { id = "e", kind = "flow.end" },
        } }"#;
        let g = match parse(&cat, text, "q") {
            Ok(g) => g,
            Err(e) => panic!("{}", e.message),
        };
        let texts: Vec<String> = run(&g, &cat, &Context::default()).iter().map(NqProblem::text).collect();
        assert!(texts.iter().any(|t| t.starts_with("E005 k:") || t.contains("ref 'nobody' is never created by this quest")), "{texts:?}");
        let beh = r#"return { beh = 1, id = "b", nodes = { { id = "s", kind = "trigger.start" } } }"#;
        let g = match parse(&cat, beh, "b") {
            Ok(g) => g,
            Err(e) => panic!("{}", e.message),
        };
        let path = "npc/b.nqasset";
        let texts: Vec<String> = run(&g, &cat, &Context { file_path: Some(path), ..Context::default() }).iter().map(NqProblem::text).collect();
        assert!(texts.iter().any(|t| t == "E006[param:roles]: a behaviour graph needs at least one role - it has nobody to drive otherwise"), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("E001: a .nqasset is a quest")), "{texts:?}");
    }
}
