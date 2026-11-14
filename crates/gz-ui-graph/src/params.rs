//! The typed editors of the Details panel: one per catalog parameter type, each a row (or a few) of the kit's
//! property grid. What comes from a list is picked, not typed; a value the list does not hold is
//! kept and shown, never corrected; a problem is a sentence under its row.
//!
//! Every row is a property grid of its own inside an id scope: a repeated label (the
//! `Character` of the second NPC) never shares a widget with the first.

use std::hash::Hash;

use egui::{Id, Pos2, Rect, Ui, Vec2};
use gz_luadata::Value;
use gz_nq::asset::{self, Cond, Graph, Task, Var};
use gz_nq::caps::{self, Caps};
use gz_nq::catalog::{self, Catalog, Kind, Param, USE_COND};
use gz_nq::{PickerType, text};
use gz_ui::grid::{self, Grid, PropFlags};
use gz_ui::menu::menu_popup;
use gz_ui::{Clip, Font, Icon, PickItem, PickList, PropertyGrid, Tone, palette};
use smol_str::SmolStr;

use crate::derived::DocData;
use crate::look;

/// What the next row says of itself.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub label: String,
    pub help: String,
    pub flags: PropFlags,
    pub depth: u32,
}

impl Row {
    #[must_use]
    pub fn new(label: &str, help: &str, depth: u32) -> Self {
        Self { label: label.to_owned(), help: help.to_owned(), flags: PropFlags::NONE, depth }
    }

    #[must_use]
    pub fn flags(mut self, f: PropFlags) -> Self {
        self.flags = self.flags | f;
        self
    }

    #[must_use]
    pub fn more_help(mut self, text_: &str) -> Self {
        if !text_.is_empty() {
            self.help =
                if self.help.is_empty() { text_.to_owned() } else { format!("{}. {text_}", self.help) };
        }
        self
    }

    fn nested(&self, label: &str, help: &str) -> Row {
        Row::new(label, help, self.depth + 1)
    }
}

/// The text a multi-line field hands to the Edit Text dialog, and what Apply leaves for it.
#[derive(Clone, Debug, Default)]
pub struct TextEdit {
    pub open: bool,
    pub code: bool,
    pub target: String,
    pub apply: bool,
    pub title: String,
    pub text: String,
    pub check: String,
}

impl TextEdit {
    pub fn open_for(&mut self, target: &str, title: &str, text_: &str, code: bool) {
        *self = TextEdit {
            open: true,
            code,
            target: target.to_owned(),
            apply: false,
            title: title.to_owned(),
            text: text_.to_owned(),
            check: String::new(),
        };
    }

    /// The text Apply left for a field, once.
    pub fn take(&mut self, target: &str) -> Option<String> {
        if !self.apply || self.target != target {
            return None;
        }
        self.apply = false;
        self.target.clear();
        Some(std::mem::take(&mut self.text))
    }
}

/// The picker popup that is up: where it hangs and its rows, gathered once when it opens.
#[derive(Clone, Debug, Default)]
pub struct PickState {
    pub anchor: Pos2,
    /// The rows of the popup that is up and the note of an empty one.
    cache: Option<(Id, Vec<PickItem>, String)>,
}

/// What the editors read and where they keep their transient state.
pub struct ParamCx<'a> {
    pub ctx: egui::Context,
    pub cat: &'a Catalog,
    pub behaviour: bool,
    pub data: &'a DocData,
    /// The document as it is: roles, nodes, the declared refs.
    pub graph: &'a Graph,
    /// The header as the Graph panel stages it: variables and tasks.
    pub vars: &'a [Var],
    pub tasks: &'a [Task],
    /// The property grid role of the pane.
    pub role: &'static str,
    pub pick: &'a mut PickState,
    pub text_edit: &'a mut TextEdit,
    /// The parameter table the current widget belongs to: a `value` asks what type the `name`
    /// beside it was declared with, an `objective_id` which task it belongs to.
    pub siblings: Option<Value>,
    /// The viewport's camera, for a position's camera button (the slot the viewport fills).
    pub camera: std::sync::Arc<gz_editor::ViewCamera>,
}

/// A row in a grid of its own, inside an id scope.
pub fn grid<R>(
    ui: &mut Ui,
    role: &'static str,
    salt: impl Hash + std::fmt::Debug,
    f: impl FnOnce(&mut Grid<'_>) -> R,
) -> R {
    ui.push_id(salt, |ui| PropertyGrid::new(role).show(ui, f)).inner
}

/// A row that only says something (a count, "Pick the task first").
pub fn text_row(ui: &mut Ui, role: &'static str, salt: impl Hash + std::fmt::Debug, row: &Row, value: &str) {
    grid(ui, role, salt, |g| {
        g.row(&row.label, &row.help, row.flags | PropFlags::READ_ONLY, row.depth, |ui| {
            grid::read_only(ui, value, Clip::End)
        });
    });
}

/// A button under the rows it adds to (Add Objective, Add Row): Ghost, indented like them.
pub fn add_button(ui: &mut Ui, label: &str, depth: u32) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(12.0 * depth as f32 + 6.0);
        clicked = gz_ui::Button::new(label).kind(gz_ui::Kind::Ghost).show(ui).clicked();
    });
    clicked
}

fn modes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Which single key of a mode table is set.
fn mode_of(v: &Value, modes: &[String]) -> String {
    if !v.is_table() {
        return String::new();
    }
    modes.iter().find(|m| v.has(m)).cloned().unwrap_or_default()
}

fn s_of(v: &Value) -> String {
    v.to_text("")
}

fn opt_str(s: &str) -> Value {
    if s.is_empty() { Value::Nil } else { Value::from(s) }
}

/// A combo over strings with an "unset" first row: returns whether it changed.
fn combo_field(ui: &mut Ui, salt: &str, cur: &mut String, items: &[String], empty: Option<&str>) -> bool {
    let mut options: Vec<&str> = Vec::with_capacity(items.len() + 2);
    if let Some(e) = empty {
        options.push(e);
    }
    options.extend(items.iter().map(String::as_str));
    // a value the list does not hold is kept and shown
    let stray = !cur.is_empty() && !items.iter().any(|i| i == cur);
    if stray {
        options.push(cur.as_str());
    }
    let mut at =
        if cur.is_empty() { 0 } else { options.iter().position(|o| *o == cur.as_str()).unwrap_or(0) };
    let owned: Vec<String> = options.iter().map(|s| (*s).to_owned()).collect();
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    if grid::combo(ui, salt, &mut at, &refs) {
        let picked = refs.get(at).copied().unwrap_or("");
        *cur = if empty.is_some() && at == 0 { String::new() } else { picked.to_owned() };
        return true;
    }
    false
}

fn combo_row(
    ui: &mut Ui,
    role: &'static str,
    salt: &str,
    row: &Row,
    cur: &mut String,
    items: &[String],
    empty: Option<&str>,
) -> bool {
    grid(ui, role, salt, |g| {
        g.row(&row.label, &row.help, row.flags, row.depth, |ui| combo_field(ui, "v", cur, items, empty))
            .unwrap_or(false)
    })
}

fn check_row(ui: &mut Ui, role: &'static str, salt: &str, row: &Row, v: &mut bool) -> bool {
    grid(ui, role, salt, |g| {
        g.row(&row.label, &row.help, row.flags | PropFlags::INLINE, row.depth, |ui| {
            grid::switch(ui, v).changed()
        })
        .unwrap_or(false)
    })
}

fn text_row_edit(ui: &mut Ui, role: &'static str, salt: &str, row: &Row, s: &mut String) -> bool {
    grid(ui, role, salt, |g| {
        g.row(&row.label, &row.help, row.flags, row.depth, |ui| grid::text_field(ui, s).changed())
            .unwrap_or(false)
    })
}

fn number_row(
    ui: &mut Ui,
    role: &'static str,
    salt: &str,
    row: &Row,
    v: &mut f32,
    speed: f32,
    unit: &str,
) -> bool {
    grid(ui, role, salt, |g| {
        g.row(&row.label, &row.help, row.flags, row.depth, |ui| grid::number(ui, v, speed, unit).changed())
            .unwrap_or(false)
    })
}

/// The first line of a text, with an ellipsis when there is more.
fn first_line(s: &str) -> String {
    match s.find('\n') {
        Some(i) => format!("{} \u{2026}", s.get(..i).unwrap_or("")),
        None => s.to_owned(),
    }
}

/// The catalog default stated again is the same as saying nothing.
fn equals_default(p: &Param, v: &Value) -> bool {
    let Some(d) = p.default.as_deref() else { return false };
    match v {
        Value::Bool(b) => d == if *b { "true" } else { "false" },
        Value::Number(n) => !d.is_empty() && text::c_atof(d) == *n,
        Value::String(s) => d == s.as_str(),
        _ => false,
    }
}

/// `true` / `false` / `12.5` / `"text"`: how a variable's value reads in its list.
#[must_use]
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
        Value::Number(n) => look::num(*n),
        other => format!("\"{}\"", other.to_text("")),
    }
}

/// `bool`, `number`, `text`.
#[must_use]
pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        _ => "?",
    }
}

/// Converts a value to a type the Type segmented control picked (0 bool, 1 number, 2 text).
#[must_use]
pub fn convert(v: &Value, to: usize) -> Value {
    match to {
        0 => Value::Bool(v.as_number().is_some_and(|n| n != 0.0)),
        1 => Value::Number(if v.as_bool() == Some(true) { 1.0 } else { 0.0 }),
        _ => Value::from(match v {
            Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
            Value::Number(n) => look::num(*n),
            _ => String::new(),
        }),
    }
}

/// A value edited as the type of the variable it belongs to.
pub fn typed_value(
    ui: &mut Ui,
    role: &'static str,
    salt: &str,
    row: &Row,
    v: &mut Value,
    as_type: &Value,
) -> bool {
    grid(ui, role, salt, |g| {
        g.row(
            &row.label,
            &row.help,
            row.flags | if as_type.is_bool() { PropFlags::INLINE } else { PropFlags::NONE },
            row.depth,
            |ui| match as_type {
                Value::Bool(_) => {
                    let mut b = v.as_bool() == Some(true);
                    let ch = grid::switch(ui, &mut b).changed();
                    if ch {
                        *v = Value::Bool(b);
                    }
                    ch
                }
                Value::Number(_) => {
                    let mut f = v.as_number().unwrap_or(0.0) as f32;
                    let ch = grid::number(ui, &mut f, 0.1, "").changed();
                    if ch {
                        *v = Value::Number(f64::from(f));
                    }
                    ch
                }
                _ => {
                    let mut s = v.as_str().unwrap_or("").to_owned();
                    let ch = grid::text_field(ui, &mut s).changed();
                    if ch {
                        *v = Value::from(s);
                    }
                    ch
                }
            },
        )
        .unwrap_or(false)
    })
}

impl ParamCx<'_> {
    fn pickers(&self) -> Option<&gz_nq::Pickers> {
        self.data.pickers.as_deref()
    }

    fn var(&self, name: &str) -> Option<&Var> {
        if name.is_empty() {
            return None;
        }
        self.vars.iter().find(|v| v.name == name)
    }

    fn open_picker(&mut self, id: Id) {
        self.pick.cache = None;
        self.pick.anchor = self.ctx.input(|i| i.pointer.interact_pos()).unwrap_or(Pos2::ZERO);
        egui::Popup::open_id(&self.ctx, id);
    }

    /// The rows a picker of a type offers, and the note of an empty one:
    /// every entry, once; the kit's list ranks them as the author types.
    fn pick_rows(&self, ty: &str) -> (Vec<PickItem>, String) {
        let mut rows: Vec<PickItem> = Vec::new();
        let doc_scoped = matches!(
            ty,
            "task_id"
                | "var_name"
                | "ref_name"
                | "node_id"
                | "quest_id"
                | "behaviour_id"
                | "role"
                | "way_path"
                | "character_id"
                | "global_name"
        );
        if doc_scoped {
            let mut push = |id: &str, extra: &str| rows.push(PickItem::new(id, id).detail(extra));
            match ty {
                "task_id" => self.graph.tasks.iter().for_each(|t| push(&t.id, "")),
                "var_name" => self.graph.vars.iter().for_each(|v| push(&v.name, "")),
                "global_name" => self.data.globals.iter().for_each(|(n, v)| push(n, &format!("= {v}"))),
                "node_id" => self.graph.nodes.iter().for_each(|n| push(&n.id, "")),
                "ref_name" => {
                    for (r, kind) in gz_nq::validate::declared_refs(self.graph, self.cat) {
                        push(&r, if matches!(kind, "squad" | "object" | "item") { kind } else { "" });
                    }
                }
                "quest_id" | "behaviour_id" => {
                    let want = ty == "behaviour_id";
                    for g in self
                        .data
                        .graphs
                        .iter()
                        .filter(|g| g.behaviour == want && g.readable && !g.id.is_empty())
                    {
                        push(&g.id, &g.path);
                    }
                }
                "role" => self.graph.roles.iter().for_each(|r| push(&r.name, "")),
                "character_id" => {
                    for c in self.data.characters.iter() {
                        let extra = if c.template {
                            "template - cannot be spawned".to_owned()
                        } else if c.name.is_empty() {
                            c.file.clone()
                        } else {
                            c.name.clone()
                        };
                        push(&c.id, &extra);
                    }
                }
                _ => self.data.way_paths.iter().for_each(|w| push(&w.id, &w.extra)),
            }
            let note = if !rows.is_empty() {
                String::new()
            } else {
                match ty {
                    "way_path" => "No way paths in the open scene - draw one with the Way tool",
                    "behaviour_id" => "No behaviour graphs in this project",
                    "role" => "This graph declares no roles",
                    "character_id" => "No characters in this project - Create > Character",
                    "global_name" => "The module declares no global variables - add one under Global",
                    _ if self.behaviour => "Nothing declared in this graph",
                    _ => "Nothing declared in this quest",
                }
                .to_owned()
            };
            return (rows, note);
        }
        let Some(t) = PickerType::from_name(ty) else {
            return (rows, "No index for this parameter".to_owned());
        };
        let Some(p) = self.pickers().filter(|p| p.available()) else {
            return (rows, "Link a game to get suggestions".to_owned());
        };
        for e in p.entries(t) {
            let label = if e.name.is_empty() { e.id.clone() } else { format!("{}  -  {}", e.id, e.name) };
            rows.push(PickItem::new(e.id.clone(), label).detail(e.extra.clone()));
        }
        let note = if rows.is_empty() { "The linked game has none".to_owned() } else { String::new() };
        (rows, note)
    }

    /// The picker popup of a field while it is up; the value picked.
    fn picker_popup(&mut self, ui: &Ui, id: Id, ty: &str) -> Option<String> {
        if !egui::Popup::is_id_open(&self.ctx, id) {
            return None;
        }
        if self.pick.cache.as_ref().is_none_or(|(cid, _, _)| *cid != id) {
            let (rows, note) = self.pick_rows(ty);
            PickList::reset(&self.ctx, ("gz-ui-graph.param_pick", id));
            self.pick.cache = Some((id, rows, note));
        }
        let (rows, note) = self.pick.cache.clone().map(|(_, r, n)| (r, n)).unwrap_or_default();
        let anchor = Rect::from_min_size(self.pick.anchor, Vec2::ZERO);
        if rows.is_empty() {
            menu_popup(&self.ctx, id, anchor, ui.layer_id(), |ui| {
                gz_ui::text::label(ui, &note, Font::Body, palette(ui.ctx()).text_disabled, Clip::End);
            });
            return None;
        }
        let list = PickList::new(("gz-ui-graph.param_pick", id), &rows).width(gz_ui::units::em(22.0));
        gz_ui::pick::pick_popup(&self.ctx, id, anchor, ui.layer_id(), list)
            .and_then(|i| rows.get(i).map(|r| r.id.clone()))
    }
}

// ---- the parameters of a kind -------------------------------------------------------------

/// Every parameter of a kind, then the ones the file has and the catalog does not (kept as Lua)
/// . Returns whether anything changed.
pub fn draw_params(ui: &mut Ui, cx: &mut ParamCx<'_>, k: &Kind, params: &mut Value, depth: u32) -> bool {
    let mut ch = false;
    if !params.is_table() {
        *params = Value::table();
    }
    let item_rows = k.has_rows() && k.params.iter().any(|p| p.ty == "item_list");
    ui.push_id(("params", k.id.as_str()), |ui| {
        if k.id == "objective.fetch" {
            ch |= fetch_params(ui, cx, k, params, depth);
        } else if item_rows {
            ch |= item_rows_params(ui, cx, k, params, depth);
            if k.id == "item.spawn" {
                ch |= item_spawn_where(ui, cx, params, depth);
            }
        } else {
            for (i, p) in k.params.iter().enumerate() {
                ui.push_id(i, |ui| ch |= draw_param(ui, cx, p, params, &k.id, depth));
            }
        }
        // parameters present in the file but unknown to the catalog stay editable as raw Lua
        let unknown: Vec<SmolStr> = params
            .fields()
            .iter()
            .filter(|(key, _)| k.param(key).is_none())
            .map(|(key, _)| key.clone())
            .collect();
        for (i, key) in unknown.iter().enumerate() {
            let mut v = params.get(key).cloned().unwrap_or_default();
            let row = Row::new(
                &format!("{key} (unknown)"),
                "Not a parameter of this kind: kept as written, in Lua",
                depth,
            )
            .flags(PropFlags::INVALID);
            if raw(ui, cx, &format!("unknown{i}"), &row, &mut v) {
                params.set(key, v);
                ch = true;
            }
        }
    });
    ch
}

/// One parameter: its typed editor, the catalog default written back as nothing.
pub fn draw_param(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    p: &Param,
    params: &mut Value,
    kind: &str,
    depth: u32,
) -> bool {
    let mut v = params.get(&p.name).cloned().unwrap_or_default();
    // a parameter another form ruled out stays visible but dead
    let ruled_out = catalog::form_ruled_out(kind, &p.name, params);
    let mut help = format!("{}: {}", p.name, p.ty);
    if let Some(d) = &p.default {
        help.push_str(&format!(" (default {d})"));
    }
    if ruled_out {
        help.push_str(&format!(". Not used: another form of {kind} is set"));
    }
    let mut row = Row::new(&look::pretty(&p.name), &help, depth);
    if p.required {
        row = row.flags(PropFlags::REQUIRED);
    }
    let keep = cx.siblings.replace(params.clone());
    let ch = ui
        .scope(|ui| {
            if ruled_out {
                ui.disable();
            }
            typed(ui, cx, &p.ty, Some(p), &p.name, &row, &mut v)
        })
        .inner;
    cx.siblings = keep;
    if ch {
        if v.is_nil() || equals_default(p, &v) {
            params.erase(&p.name);
        } else {
            params.set(&p.name, v);
        }
    }
    ch
}

/// The editor of a parameter type.
#[allow(clippy::too_many_lines)]
pub fn typed(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    ty: &str,
    p: Option<&Param>,
    salt: &str,
    row: &Row,
    v: &mut Value,
) -> bool {
    let role = cx.role;
    match ty {
        "text" => draw_text(ui, cx, salt, row, v, true),
        "lua" => lua(ui, cx, salt, row, v),
        "bool" => {
            let set = v.is_bool();
            let mut b = if set {
                v.as_bool() == Some(true)
            } else {
                p.and_then(|p| p.default.as_deref()) == Some("true")
            };
            let mut ch = false;
            grid(ui, role, salt, |g| {
                let flags =
                    row.flags | PropFlags::INLINE | if set { PropFlags::MODIFIED } else { PropFlags::NONE };
                if g.row(&row.label, &row.help, flags, row.depth, |ui| grid::switch(ui, &mut b).changed())
                    .unwrap_or(false)
                {
                    *v = Value::Bool(b);
                    ch = true;
                }
                if set && g.was_reset() {
                    *v = Value::Nil;
                    ch = true;
                }
            });
            ch
        }
        "int" | "float" => {
            let is_int = ty == "int";
            let def = p.and_then(|p| p.default.as_deref()).map_or(0.0, text::c_atof);
            let mut d = v.as_number().unwrap_or(def);
            let set = !v.is_nil();
            let mut ch = false;
            grid(ui, role, salt, |g| {
                let flags = row.flags | if set { PropFlags::MODIFIED } else { PropFlags::NONE };
                let changed = g
                    .row(&row.label, &row.help, flags, row.depth, |ui| {
                        if is_int {
                            let mut i = d as i32;
                            let c = grid::integer(ui, &mut i, 0.25, "").changed();
                            if c {
                                d = f64::from(i);
                            }
                            c
                        } else {
                            let mut f = d as f32;
                            let c = grid::number(ui, &mut f, 0.1, "").changed();
                            if c {
                                d = f64::from(f);
                            }
                            c
                        }
                    })
                    .unwrap_or(false);
                if changed {
                    if let Some(min) = p.and_then(|p| p.min) {
                        d = d.max(min);
                    }
                    if let Some(max) = p.and_then(|p| p.max) {
                        d = d.min(max);
                    }
                    *v = Value::Number(d);
                    ch = true;
                }
                if set && g.was_reset() {
                    *v = Value::Nil;
                    ch = true;
                }
            });
            ch
        }
        "enum" | "relation" => {
            let items = if ty == "relation" {
                modes(&["enemy", "neutral", "friend"])
            } else {
                p.map(|p| p.enums.clone()).unwrap_or_default()
            };
            let mut cur = v.as_str().unwrap_or("").to_owned();
            let empty = p
                .and_then(|p| p.default.as_deref())
                .map_or_else(|| "(unset)".to_owned(), |d| format!("(default: {d})"));
            if combo_row(ui, role, salt, row, &mut cur, &items, Some(&empty)) {
                *v = opt_str(&cur);
                return true;
            }
            false
        }
        "duration" => duration(ui, cx, salt, row, v),
        "npc_ref" => npc_ref(ui, cx, salt, row, v, false, false),
        "target_ref" => npc_ref(ui, cx, salt, row, v, true, false),
        "object_ref" | "squad_ref" => object_ref(ui, cx, salt, row, v, ty == "object_ref"),
        "objective_id" => objective_id(ui, cx, salt, row, v),
        "kill_target" => npc_ref(ui, cx, salt, row, v, false, true),
        "place" => place(ui, cx, salt, row, v),
        "destination" => destination(ui, cx, salt, row, v),
        "role" => who(ui, cx, salt, row, v),
        "role_or_actor" => role_target(ui, cx, salt, row, v, false),
        "attack_target" => role_target(ui, cx, salt, row, v, true),
        "role_bindings" => role_bindings(ui, cx, salt, row, v),
        "npc_list" => npc_list(ui, cx, salt, row, v),
        "spawn_spec" => spawn_spec(ui, cx, salt, row, v),
        "item_list" => item_list(ui, cx, salt, row, v, true, None),
        "loot_spawn" => loot_spawn(ui, cx, salt, row, v),
        "cases_cond" => cases(ui, cx, salt, row, v, false),
        "cases_weight" => cases(ui, cx, salt, row, v, true),
        "cond_list" => cond_value(ui, cx, salt, row, v, 1),
        "var_name" => {
            let mut s = v.to_text("");
            if var_name_combo(ui, cx, salt, row, &mut s) {
                *v = opt_str(&s);
                return true;
            }
            false
        }
        "value" => {
            // `value` sits next to the `name` it is assigned to or compared with: the variable's
            // declared type decides the widget
            let name = cx
                .siblings
                .as_ref()
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if let Some(var) = cx.var(&name) {
                let as_type = var.value.clone();
                return typed_value(ui, role, salt, row, v, &as_type);
            }
            scalar_value(ui, cx, salt, row, v)
        }
        "count_or_all" => {
            let mut s = match &*v {
                Value::Number(n) => format!("{}", *n as i64),
                other => other.to_text(""),
            };
            if text_row_edit(ui, role, salt, row, &mut s) {
                let t = s.trim();
                *v = if t.is_empty() {
                    Value::Nil
                } else if t == "all" {
                    Value::from("all")
                } else {
                    Value::Number(text::c_atof(t))
                };
                return true;
            }
            false
        }
        _ => {
            let mut s = v.to_text("");
            if picked(ui, cx, salt, row, ty, &mut s) {
                *v = opt_str(&s);
                return true;
            }
            false
        }
    }
}

/// A string with a picker beside it, marked when the index does not know it.
pub fn picked(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, ty: &str, s: &mut String) -> bool {
    let pt = PickerType::from_name(ty);
    let unknown = !s.is_empty()
        && pt.is_some_and(|t| cx.pickers().is_some_and(|p| !p.entries(t).is_empty() && !p.known(t, s)));
    let flags = row.flags | if unknown { PropFlags::INVALID } else { PropFlags::NONE };
    let pid = ui.make_persistent_id(("gz-ui-graph.pick", salt));
    let role = cx.role;
    let mut ch = false;
    let mut open = false;
    grid(ui, role, salt, |g| {
        ch |= g
            .row(&row.label, &row.help, flags, row.depth, |ui| grid::text_field(ui, s).changed())
            .unwrap_or(false);
        open = g.trailing(Icon::More, "Pick from the list");
        if unknown {
            let list = pt.map_or_else(|| ty.to_owned(), |t| t.name().to_owned());
            let why =
                format!("Nothing named '{s}' is in the {list} list: place it first, or pick one that exists");
            g.message(Tone::Error, &why);
        }
    });
    if open {
        cx.open_picker(pid);
    }
    if let Some(v) = cx.picker_popup(ui, pid, ty) {
        *s = v;
        ch = true;
    }
    ch
}

/// A text: one line typed in its row, a longer one shown by its first line and edited in a
/// dialog; a text per language puts each language on a row of its own.
pub fn draw_text(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    multiline: bool,
) -> bool {
    let role = cx.role;
    let mut ch = false;
    let target = format!("{:?}/{salt}", ui.scope_id());
    if v.is_table() {
        let mut add_eng = false;
        let mut single = false;
        grid(ui, role, salt, |g| {
            g.row(&row.label, &row.help, row.flags, row.depth, |ui| {
                grid::read_only(ui, "(per language)", Clip::End)
            });
            if !v.has("eng") && g.trailing(Icon::Plus, "Add an English text") {
                add_eng = true;
            }
            if g.trailing(Icon::Language, "One text for every language") {
                single = true;
            }
        });
        if add_eng {
            v.set("eng", "");
            ch = true;
        }
        if single {
            *v = Value::from(text::preview(v));
            return true;
        }
        let keys: Vec<SmolStr> = v.fields().iter().map(|(k, _)| k.clone()).collect();
        for (i, key) in keys.iter().enumerate() {
            let t = format!("{target}/{key}");
            let mut s = v.get(key).map(s_of).unwrap_or_default();
            let mut edited = false;
            if let Some(applied) = cx.text_edit.take(&t) {
                s = applied;
                edited = true;
            }
            let lr = row.nested(key, &row.help);
            let what = format!("{} ({key})", row.label);
            edited |= line_field(ui, cx, &format!("{salt}/lang{i}"), &lr, &mut s, multiline, &t, &what);
            if edited {
                v.set(key, s);
                ch = true;
            }
        }
        return ch;
    }
    let mut s = v.to_text("");
    if let Some(applied) = cx.text_edit.take(&target) {
        s = applied;
        *v = Value::from(s.clone());
        ch = true;
    }
    let mut per_lang = false;
    let has_lines = s.contains('\n');
    let mut edit = false;
    grid(ui, role, salt, |g| {
        let changed = g
            .row(&row.label, &row.help, row.flags, row.depth, |ui| {
                if has_lines {
                    grid::read_only(ui, &first_line(&s), Clip::End);
                    false
                } else {
                    grid::text_field(ui, &mut s).changed()
                }
            })
            .unwrap_or(false);
        if changed {
            *v = Value::from(s.clone());
            ch = true;
        }
        if g.trailing(Icon::Language, "A text per language") {
            per_lang = true;
        }
        if (multiline || has_lines) && g.trailing(Icon::Rename, "Edit the whole text") {
            edit = true;
        }
    });
    if edit {
        cx.text_edit.open_for(&target, &row.label, &s, false);
    }
    if per_lang {
        let mut t = Value::table();
        t.set("rus", s);
        *v = t;
        ch = true;
    }
    ch
}

#[allow(clippy::too_many_arguments)]
fn line_field(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    s: &mut String,
    multiline: bool,
    target: &str,
    what: &str,
) -> bool {
    let role = cx.role;
    let has_lines = s.contains('\n');
    let mut edit = false;
    let ch = grid(ui, role, salt, |g| {
        let c = g
            .row(&row.label, &row.help, row.flags, row.depth, |ui| {
                if has_lines {
                    grid::read_only(ui, &first_line(s), Clip::End);
                    false
                } else {
                    grid::text_field(ui, s).changed()
                }
            })
            .unwrap_or(false);
        if (multiline || has_lines) && g.trailing(Icon::Rename, "Edit the whole text") {
            edit = true;
        }
        c
    });
    if edit {
        cx.text_edit.open_for(target, what, s, false);
    }
    ch
}

/// A number and its unit.
pub fn duration(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let units = modes(&["seconds", "game_minutes", "game_hours"]);
    let mut unit = mode_of(v, &units);
    let mut num = if unit.is_empty() { 0.0 } else { v.get_number(&unit, 0.0) };
    let mut ch = false;
    grid(cx_ui(ui), cx.role, salt, |g| {
        g.row(&row.label, &row.help, row.flags, row.depth, |ui| {
            let w = ui.available_width();
            let unit_w = (w * 0.55).min(gz_ui::units::em(8.0));
            let mut f = num as f32;
            ui.scope(|ui| {
                ui.set_max_width((w - unit_w - 6.0).max(gz_ui::units::em(3.0)));
                if grid::number(ui, &mut f, 0.1, "").changed() {
                    num = f64::from(f);
                    ch = true;
                }
            });
            ui.add_space(6.0);
            ui.scope(|ui| {
                ui.set_max_width(unit_w);
                let mut u = if unit.is_empty() { "seconds".to_owned() } else { unit.clone() };
                if combo_field(ui, "u", &mut u, &units, None) {
                    unit = u;
                    ch = true;
                }
            });
        });
    });
    if ch {
        if unit.is_empty() {
            "seconds".clone_into(&mut unit);
        }
        let mut t = Value::table();
        t.set(&unit, num);
        *v = t;
    }
    ch
}

fn cx_ui(ui: &mut Ui) -> &mut Ui {
    ui
}

/// The value field of a mode table: `story = "..."` picked by its type.
fn pick_type_of(mode: &str) -> &str {
    match mode {
        "ref" => "ref_name",
        "path" => "way_path",
        "story" => "story_id",
        "character" => "character_id",
        other => other,
    }
}

/// The row of a value that takes one of several forms: the form picked here, what it needs on
/// rows under it.
fn mode_row(
    ui: &mut Ui,
    role: &'static str,
    salt: &str,
    row: &Row,
    mode: &mut String,
    list: &[String],
) -> bool {
    combo_row(ui, role, salt, row, mode, list, Some("(unset)"))
}

fn fresh_pos(level: &str, radius: Option<f64>) -> Value {
    let mut t = Value::table();
    t.set("level", level);
    t.set("pos", Value::from(vec![Value::Number(0.0), Value::Number(0.0), Value::Number(0.0)]));
    if let Some(r) = radius {
        t.set("radius", r);
    }
    t
}

/// A creature or a spot a quest points at: the form, then its value.
pub fn npc_ref(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    target: bool,
    kill: bool,
) -> bool {
    let mut list = modes(&["character", "story", "ref", "profile", "community"]);
    if target {
        list.extend(modes(&["smart", "restrictor", "pos"]));
    }
    if kill {
        list.push("spawn".to_owned());
    }
    let mut mode = mode_of(v, &list);
    if mode.is_empty() && target && v.is_table() && v.has("pos") {
        "pos".clone_into(&mut mode);
    }
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, row, &mut m, &list) {
        ch = true;
        *v = if m.is_empty() {
            Value::Nil
        } else if m == "spawn" {
            let mut s = Value::table();
            s.set("section", "");
            let mut t = Value::table();
            t.set("spawn", s);
            t
        } else if m == "pos" {
            fresh_pos(&cx.data.level, Some(5.0))
        } else {
            let mut t = Value::table();
            t.set(&m, "");
            t
        };
        mode = m;
    }
    let d = row.depth + 1;
    match mode.as_str() {
        "" => {}
        "pos" => ch |= position(ui, cx, &format!("{salt}/pos"), d, v),
        "spawn" => {
            let mut sp = v.get("spawn").cloned().unwrap_or_default();
            if spawn_spec(ui, cx, &format!("{salt}/spawn"), &Row::new("Spawn", "", d), &mut sp) {
                v.set("spawn", sp);
                ch = true;
            }
        }
        m => {
            let mut s = v.get_string(m, "");
            if picked(
                ui,
                cx,
                &format!("{salt}/value"),
                &Row::new(&look::pretty(m), "", d),
                pick_type_of(m),
                &mut s,
            ) {
                v.set(m, s);
                ch = true;
            }
            if m == "community" {
                let mut lvl = v.get_string("level", "");
                if picked(
                    ui,
                    cx,
                    &format!("{salt}/level"),
                    &Row::new("Level", "Only NPCs of the community on this level; unset: any level", d),
                    "level",
                    &mut lvl,
                ) {
                    if lvl.is_empty() {
                        v.erase("level");
                    } else {
                        v.set("level", lvl);
                    }
                    ch = true;
                }
            }
        }
    }
    ch
}

/// One concrete object: a story id, a ref, and for an object a character.
pub fn object_ref(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    character: bool,
) -> bool {
    let mut list = modes(&["story", "ref"]);
    if character {
        list.push("character".to_owned());
    }
    let mut mode = mode_of(v, &list);
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, row, &mut m, &list) {
        ch = true;
        *v = if m.is_empty() {
            Value::Nil
        } else {
            let mut t = Value::table();
            t.set(&m, "");
            t
        };
        mode = m;
    }
    if !mode.is_empty() {
        let mut s = v.get_string(&mode, "");
        if picked(
            ui,
            cx,
            &format!("{salt}/value"),
            &row.nested(&look::pretty(&mode), ""),
            pick_type_of(&mode),
            &mut s,
        ) {
            v.set(&mode, s);
            ch = true;
        }
    }
    ch
}

/// The steps of whichever task the sibling `task` (or `spot_task`) names.
pub fn objective_id(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let owner = cx.siblings.as_ref().and_then(|s| s.get("task").or_else(|| s.get("spot_task"))).cloned();
    let task = owner.as_ref().and_then(Value::as_str).and_then(|id| cx.tasks.iter().find(|t| t.id == id));
    let ids: Vec<String> =
        task.map(|t| t.objectives.iter().map(|o| o.id.clone()).collect()).unwrap_or_default();
    match task {
        None => {
            let msg = if owner.as_ref().is_some_and(Value::is_string) {
                "Pick a task that exists first"
            } else {
                "Pick the task first"
            };
            text_row(ui, cx.role, salt, row, msg);
            false
        }
        Some(t) if ids.is_empty() => {
            text_row(ui, cx.role, salt, row, &format!("The task '{}' has no objectives yet", t.id));
            false
        }
        Some(_) => {
            let mut cur = v.as_str().unwrap_or("").to_owned();
            if combo_row(ui, cx.role, salt, row, &mut cur, &ids, Some("(unset)")) {
                *v = opt_str(&cur);
                return true;
            }
            false
        }
    }
}

/// A place: a position, a zone, a smart, a ref, or a point ahead of the player.
pub fn place(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let list = modes(&["pos", "restrictor", "smart", "ref", "actor"]);
    let mut mode = mode_of(v, &list);
    if mode.is_empty() && v.is_table() && v.has("level") {
        "pos".clone_into(&mut mode);
    }
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, row, &mut m, &list) {
        ch = true;
        *v = match m.as_str() {
            "" => Value::Nil,
            "pos" => fresh_pos(&cx.data.level, Some(5.0)),
            "actor" => {
                let mut t = Value::table();
                t.set("actor", true);
                t.set("distance", 2.0);
                t
            }
            other => {
                let mut t = Value::table();
                t.set(other, "");
                t
            }
        };
        mode = m;
    }
    let d = row.depth + 1;
    match mode.as_str() {
        "" => {}
        "pos" => ch |= position(ui, cx, &format!("{salt}/pos"), d, v),
        "actor" => {
            let mut dist = v.get_number("distance", 0.0) as f32;
            let r = Row::new(
                "Ahead, m",
                "Resolved when the action runs: that many metres in front of the player, snapped to the navmesh. 0 = at their feet",
                d,
            );
            if number_row(ui, cx.role, &format!("{salt}/distance"), &r, &mut dist, 0.1, "m") {
                if dist <= 0.0 {
                    v.erase("distance");
                } else {
                    v.set("distance", f64::from(dist));
                }
                ch = true;
            }
        }
        m => {
            let mut s = v.get_string(m, "");
            if picked(
                ui,
                cx,
                &format!("{salt}/value"),
                &Row::new(&look::pretty(m), "", d),
                pick_type_of(m),
                &mut s,
            ) {
                v.set(m, s);
                ch = true;
            }
        }
    }
    ch
}

/// A spot spelled out by hand: its level, the point, the radius.
pub fn position(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, depth: u32, v: &mut Value) -> bool {
    let mut ch = false;
    let mut lvl = v.get_string("level", "");
    if picked(
        ui,
        cx,
        &format!("{salt}/level"),
        &Row::new("Level", "The level the point is on", depth),
        "level",
        &mut lvl,
    ) {
        v.set("level", lvl);
        ch = true;
    }
    let mut xyz = [0.0f32; 3];
    if let Some(pos) = v.get("pos").filter(|p| p.is_table()) {
        for (i, c) in pos.arr().iter().take(3).enumerate() {
            if let Some(slot) = xyz.get_mut(i) {
                *slot = c.to_number(0.0) as f32;
            }
        }
    }
    let camera = cx.camera.clone();
    let mut from_camera = false;
    let changed = grid(ui, cx.role, (salt, "xyz"), |g| {
        let edited = g
            .row(
                "Position",
                "The point, in the level's coordinates. The camera icon takes the viewport camera's position",
                PropFlags::NONE,
                depth,
                |ui| grid::vec3_field(ui, "xyz", &mut xyz, 0.1),
            )
            .unwrap_or(false);
        // the camera button only where the viewport fills the slot
        if camera.filled() && g.trailing(Icon::Perspective, "Use the viewport camera's position and level") {
            from_camera = true;
        }
        edited
    });
    if changed {
        v.set("pos", Value::from(xyz.iter().map(|c| Value::Number(f64::from(*c))).collect::<Vec<_>>()));
        ch = true;
    }
    if from_camera && let Some(pose) = camera.get() {
        v.set(
            "pos",
            Value::from(pose.position.iter().map(|c| Value::Number(f64::from(*c))).collect::<Vec<_>>()),
        );
        if let Some(level) = pose.level {
            v.set("level", level);
        }
        ch = true;
    }
    let mut r = v.get_number("radius", 5.0) as f32;
    if number_row(
        ui,
        cx.role,
        &format!("{salt}/radius"),
        &Row::new("Radius, m", "How close the player has to get, in metres. 5 when not set", depth),
        &mut r,
        0.1,
        "m",
    ) {
        v.set("radius", f64::from(r));
        ch = true;
    }
    ch
}

/// Where a step sends people: a way path first - the best destination.
pub fn destination(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let mut list = modes(&["path", "ref", "story", "character"]);
    // a role is a behaviour graph's cast; a quest only sees one it has to correct
    if cx.behaviour || v.has("role") {
        list.push("role".to_owned());
    }
    list.extend(modes(&["smart", "restrictor", "pos", "actor"]));
    let mut mode = mode_of(v, &list);
    if mode.is_empty() && v.is_table() && v.has("level") {
        "pos".clone_into(&mut mode);
    }
    let row = row.clone().more_help(
        "path: a way path drawn with the Way tool - the best destination. ref: something the caller spawned; the step waits for it. story: an object of the level. character: wherever that character of the project stands. role: wherever that role stands. smart / restrictor / pos / actor: a place",
    );
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, &row, &mut m, &list) {
        ch = true;
        *v = match m.as_str() {
            "" => Value::Nil,
            "pos" => fresh_pos(&cx.data.level, None),
            "actor" => {
                let mut t = Value::table();
                t.set("actor", true);
                t
            }
            other => {
                let mut t = Value::table();
                t.set(other, "");
                t
            }
        };
        mode = m;
    }
    let d = row.depth + 1;
    match mode.as_str() {
        "" | "actor" => {}
        "pos" => ch |= position(ui, cx, &format!("{salt}/pos"), d, v),
        m => {
            let mut s = v.get_string(m, "");
            if picked(
                ui,
                cx,
                &format!("{salt}/value"),
                &Row::new(&look::pretty(m), "", d),
                pick_type_of(m),
                &mut s,
            ) {
                v.set(m, s);
                ch = true;
            }
        }
    }
    ch
}

/// `who`: one role or several, ticked from what the graph declares.
pub fn who(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let roles = &cx.graph.roles;
    let mut picked_roles: Vec<String> = match v {
        Value::String(s) if !s.is_empty() => vec![s.to_string()],
        Value::Table(t) => t.arr.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let stray: Vec<String> =
        picked_roles.iter().filter(|p| !roles.iter().any(|r| r.name == **p)).cloned().collect();
    let flags = row.flags | if stray.is_empty() { PropFlags::NONE } else { PropFlags::INVALID };
    let mut ch = false;
    grid(ui, cx.role, salt, |g| {
        g.row(&row.label, &row.help, flags, row.depth, |ui| {
            // a few roles: each one a box of its own on the row, ticked where it is picked
            ui.horizontal_wrapped(|ui| {
                if roles.is_empty() && stray.is_empty() {
                    gz_ui::text::label(
                        ui,
                        "Declare a role in the Graph panel first",
                        Font::Body,
                        palette(ui.ctx()).text_disabled,
                        Clip::End,
                    );
                }
                for r in roles {
                    let mut on = picked_roles.contains(&r.name);
                    let cap = if r.many { format!("{} (group)", r.name) } else { r.name.clone() };
                    if gz_ui::Check::new(&mut on, &cap).show(ui).changed() {
                        if on {
                            picked_roles.push(r.name.clone());
                        } else {
                            picked_roles.retain(|p| *p != r.name);
                        }
                        ch = true;
                    }
                }
                // a stray name can still be unticked
                for st in &stray {
                    let mut on = true;
                    let cap = format!("{st} (not a role)");
                    if gz_ui::Check::new(&mut on, &cap).show(ui).changed() && !on {
                        picked_roles.retain(|p| p != st);
                        ch = true;
                    }
                }
            });
        });
        if !stray.is_empty() {
            g.message(Tone::Error, &format!("Not a role of this graph: {}", stray.join(", ")));
        }
    });
    if ch {
        *v = match picked_roles.as_slice() {
            [] => Value::Nil,
            [one] => Value::from(one.as_str()),
            many => Value::from(many.iter().map(|s| Value::from(s.as_str())).collect::<Vec<_>>()),
        };
    }
    ch
}

/// Who a follow walks after, or who an attack is aimed at.
pub fn role_target(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    attack: bool,
) -> bool {
    let mut list = modes(&["actor", "ref", "story", "character"]);
    if cx.behaviour || v.has("role") {
        list.insert(1, "role".to_owned());
    }
    if attack {
        list.extend(modes(&["community", "profile"]));
    }
    let mut mode = mode_of(v, &list);
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, row, &mut m, &list) {
        ch = true;
        *v = match m.as_str() {
            "" => Value::Nil,
            "actor" => {
                let mut t = Value::table();
                t.set("actor", true);
                t
            }
            other => {
                let mut t = Value::table();
                t.set(other, "");
                t
            }
        };
        mode = m;
    }
    if !mode.is_empty() && mode != "actor" {
        let mut s = v.get_string(&mode, "");
        if picked(
            ui,
            cx,
            &format!("{salt}/value"),
            &row.nested(&look::pretty(&mode), ""),
            pick_type_of(&mode),
            &mut s,
        ) {
            v.set(&mode, s);
            ch = true;
        }
    }
    ch
}

/// What a caller hands the graph it runs: one row per role THAT graph declares, read from the
/// asset the sibling `asset` names.
pub fn role_bindings(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let asset_id = cx.siblings.as_ref().map(|s| s.get_string("asset", "")).unwrap_or_default();
    let mut roles: Vec<gz_nq::Role> = Vec::new();
    let mut error = String::new();
    if !asset_id.is_empty() && !asset_id.contains('.') {
        match cx.data.graphs.iter().find(|g| g.behaviour && g.readable && g.id == asset_id) {
            Some(g) => roles.clone_from(&g.roles),
            None => error = format!("No behaviour graph called '{asset_id}' in this project"),
        }
    }
    let flags = row.flags | if error.is_empty() { PropFlags::NONE } else { PropFlags::INVALID };
    grid(ui, cx.role, salt, |g| {
        let shown = if asset_id.is_empty() {
            "Pick the asset first - its roles appear here".to_owned()
        } else if error.is_empty() {
            look::plural(roles.len(), "role", "roles")
        } else {
            String::new()
        };
        g.row(&row.label, &row.help, flags | PropFlags::READ_ONLY, row.depth, |ui| {
            grid::read_only(ui, &shown, Clip::End)
        });
        if !error.is_empty() {
            g.message(Tone::Error, &error);
        }
    });
    let mut ch = false;
    for (i, r) in roles.iter().enumerate() {
        let mut binding = v.get(&r.name).cloned().unwrap_or_default();
        let mut help = text::preview(&r.title);
        let add = |h: &mut String, s: &str| {
            if h.is_empty() {
                s.clone_into(h);
            } else {
                h.push_str(". ");
                h.push_str(s);
            }
        };
        if r.many {
            add(&mut help, "A group");
        }
        if !r.default.is_nil() {
            add(&mut help, "Has a default");
        } else if r.optional {
            add(&mut help, "Optional");
        }
        if npc_ref(
            ui,
            cx,
            &format!("{salt}/bind{i}"),
            &row.nested(&r.name, &help),
            &mut binding,
            false,
            false,
        ) {
            if binding.is_empty() {
                v.erase(&r.name);
            } else {
                v.set(&r.name, binding);
            }
            if v.is_empty() {
                *v = Value::Nil;
            }
            ch = true;
        }
    }
    // a binding for a role the asset no longer declares is shown, not swallowed
    if !(roles.is_empty() && asset_id.is_empty()) {
        let keys: Vec<SmolStr> = v.fields().iter().map(|(k, _)| k.clone()).collect();
        for (k, key) in keys.iter().enumerate() {
            let known =
                (roles.is_empty() && !error.is_empty()) || roles.iter().any(|r| r.name == key.as_str());
            if known {
                continue;
            }
            let mut drop = false;
            grid(ui, cx.role, (salt, "stray", k), |g| {
                g.row(key, "", PropFlags::INVALID | PropFlags::READ_ONLY, row.depth + 1, |ui| {
                    grid::read_only(ui, "", Clip::End)
                });
                drop = g.trailing(Icon::Delete, "Remove the binding");
                g.message(Tone::Warning, "The asset declares no such role");
            });
            if drop {
                v.erase(key);
                ch = true;
                break;
            }
        }
    }
    ch
}

/// spawn.npc: one row per NPC - a character of the project or a section.
#[allow(clippy::too_many_lines)]
pub fn npc_list(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    if !v.is_table() {
        *v = Value::table();
    }
    let mut ch = false;
    let n = v.arr().len();
    text_row(
        ui,
        cx.role,
        salt,
        &row.clone().more_help("A row is one NPC: a project character or a spawn section"),
        &format!("{n} NPC{}", if n == 1 { "" } else { "s" }),
    );
    let d = row.depth + 1;
    let (mut remove, mut up, mut down) = (None, None, None);
    for i in 0..n {
        let Some(t) = v.table_mut() else { break };
        let Some(mut r) = t.arr.get(i).cloned() else { continue };
        if !r.is_table() {
            r = Value::table();
        }
        let by_section = r.has("section") && !r.has("character");
        let mut cur = usize::from(by_section);
        let mut changed_mode = false;
        grid(ui, cx.role, (salt, i, "head"), |g| {
            g.row(&format!("NPC {}", i + 1), "character: an NPC of this project (Create > Character). section: any spawn section the game has", PropFlags::NONE, d, |ui| {
                changed_mode = gz_ui::segmented(ui, &mut cur, &["Character", "Section"], None);
            });
            if g.trailing(Icon::Delete, "Remove the NPC") {
                remove = Some(i);
            }
            if i + 1 < n && g.trailing(Icon::MoveDown, "Move down") {
                down = Some(i);
            }
            if i > 0 && g.trailing(Icon::MoveUp, "Move up") {
                up = Some(i);
            }
        });
        let mut row_ch = false;
        if changed_mode && (cur == 1) != by_section {
            let next = if cur == 1 { "section" } else { "character" };
            // a row names exactly one: the other name goes
            r.erase(if next == "section" { "character" } else { "section" });
            if !r.has(next) {
                r.set(next, "");
            }
            // the default of `unique` follows the mode; only what differs from it is kept
            if r.get("unique").and_then(Value::as_bool) == Some(next == "character") {
                r.erase("unique");
            }
            row_ch = true;
        }
        let section_now = r.has("section") && !r.has("character");
        let key = if section_now { "section" } else { "character" };
        let mut s = r.get_string(key, "");
        let who_row = if section_now {
            Row::new("Section", "The spawn section of the NPC", d + 1)
        } else {
            Row::new("Character", "The project's character: its id", d + 1)
        };
        if picked(
            ui,
            cx,
            &format!("{salt}/{i}/who"),
            &who_row,
            if section_now { "spawn_section" } else { "character_id" },
            &mut s,
        ) {
            r.erase(if section_now { "character" } else { "section" });
            r.set(key, s);
            row_ch = true;
        }
        let mut rf = r.get_string("ref", "");
        let ref_row = Row::new(
            "Ref",
            "The name this NPC is known by from here on: { ref = ... } in a later node, a role of behaviour.run, a kill objective. Type a new name or pick one",
            d + 1,
        );
        if picked(ui, cx, &format!("{salt}/{i}/ref"), &ref_row, "ref_name", &mut rf) {
            if rf.is_empty() {
                r.erase("ref");
            } else {
                r.set("ref", rf);
            }
            row_ch = true;
        }
        let unique_def = !section_now;
        let mut unique = r.get_bool("unique", unique_def);
        let u_row = Row::new(
            "Unique",
            "Every NPC of this kind already in the world is removed first, so there is never a second one. On by default for a character, off for a section",
            d + 1,
        );
        if check_row(ui, cx.role, &format!("{salt}/{i}/unique"), &u_row, &mut unique) {
            if unique == unique_def {
                r.erase("unique");
            } else {
                r.set("unique", unique);
            }
            row_ch = true;
        }
        let mut dead = r.get_bool("dead", false);
        if check_row(
            ui,
            cx.role,
            &format!("{salt}/{i}/dead"),
            &Row::new("Dead", "Spawned as a corpse", d + 1),
            &mut dead,
        ) {
            if dead {
                r.set("dead", true);
            } else {
                r.erase("dead");
            }
            row_ch = true;
        }
        let mut yaw = r.get_number("yaw", 0.0) as f32;
        if number_row(
            ui,
            cx.role,
            &format!("{salt}/{i}/yaw"),
            &Row::new("Yaw", "Where it faces, in degrees", d + 1),
            &mut yaw,
            1.0,
            "\u{b0}",
        ) {
            if yaw == 0.0 {
                r.erase("yaw");
            } else {
                r.set("yaw", f64::from(yaw));
            }
            row_ch = true;
        }
        if row_ch && let Some(slot) = v.table_mut().and_then(|t| t.arr.get_mut(i)) {
            *slot = r;
            ch = true;
        }
    }
    if let Some(t) = v.table_mut() {
        if let Some(i) = remove {
            t.arr.remove(i);
            ch = true;
        } else if let Some(i) = up.filter(|&i| i > 0) {
            t.arr.swap(i, i - 1);
            ch = true;
        } else if let Some(i) = down.filter(|&i| i + 1 < t.arr.len()) {
            t.arr.swap(i, i + 1);
            ch = true;
        }
    }
    if add_button(ui, "Add NPC", d) {
        let mut r = Value::table();
        r.set("character", "");
        v.push(r);
        ch = true;
    }
    ch
}

/// A squad to create.
pub fn spawn_spec(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    if !v.is_table() {
        *v = Value::table();
    }
    let d = row.depth;
    let mut ch = false;
    let mut s = v.get_string("section", "");
    if picked(
        ui,
        cx,
        &format!("{salt}/section"),
        &Row::new("Squad section", "Which squad to create; its section decides how many and who they are", d)
            .flags(PropFlags::REQUIRED),
        "squad_section",
        &mut s,
    ) {
        v.set("section", s);
        ch = true;
    }
    let mut smart = v.get_string("smart", "");
    if picked(
        ui,
        cx,
        &format!("{salt}/smart"),
        &Row::new(
            "Smart",
            "Optional: the smart that owns them. Empty and with a place set, the nearest one is used",
            d,
        ),
        "smart",
        &mut smart,
    ) {
        // erased when empty: a written "" reads as "a smart was named"
        if smart.is_empty() {
            v.erase("smart");
        } else {
            v.set("smart", smart);
        }
        ch = true;
    }
    let mut pl = v.get("place").cloned().unwrap_or_default();
    if place(
        ui,
        cx,
        &format!("{salt}/place"),
        &Row::new(
            "Appear at",
            "Where they appear. Empty = on the smart. A zone scatters them across it, a point puts them all on it",
            d,
        ),
        &mut pl,
    ) {
        if pl.is_nil() {
            v.erase("place");
        } else {
            v.set("place", pl.clone());
        }
        ch = true;
    }
    if pl.is_table() && pl.has("restrictor") {
        let mut spread = v.get_number("spread", 1.0) as f32;
        if number_row(
            ui,
            cx.role,
            &format!("{salt}/spread"),
            &Row::new(
                "Spread",
                "How much of the zone to spawn in: 0.5 keeps them off the edges. The zone itself does not change",
                d,
            ),
            &mut spread,
            0.01,
            "",
        ) {
            spread = spread.clamp(0.05, 1.0);
            if spread >= 0.999 {
                v.erase("spread");
            } else {
                v.set("spread", f64::from(spread));
            }
            ch = true;
        }
    }
    let mut pen = v.get_string("restrictor", "");
    if picked(
        ui,
        cx,
        &format!("{salt}/pen"),
        &Row::new(
            "Penned in",
            "A zone they are penned into. They keep working, they just cannot wander off",
            d,
        ),
        "restrictor",
        &mut pen,
    ) {
        if pen.is_empty() {
            v.erase("restrictor");
        } else {
            v.set("restrictor", pen);
        }
        ch = true;
    }
    let mut rf = v.get_string("ref", "");
    if picked(
        ui,
        cx,
        &format!("{salt}/ref"),
        &Row::new("Ref", "Other nodes point at this squad by that name", d),
        "ref_name",
        &mut rf,
    ) {
        if rf.is_empty() {
            v.erase("ref");
        } else {
            v.set("ref", rf);
        }
        ch = true;
    }
    let mut hold = v.get_bool("hold", true);
    if check_row(
        ui,
        cx.role,
        &format!("{salt}/hold"),
        &Row::new("Hold", "Pins the squad to its smart, so the simulation cannot send it elsewhere", d),
        &mut hold,
    ) {
        v.set("hold", hold);
        ch = true;
    }
    ch
}

/// Whether a row schema carries anything beyond what to make and how many.
fn rows_have_state(k: Option<&Kind>) -> bool {
    k.is_some_and(|k| k.rows.iter().any(|r| r.name != "section" && r.name != "count"))
}

/// One line for a row that carries state.
fn row_state_summary(e: &Value) -> String {
    let mut s = String::new();
    for key in ["condition", "ammo", "ammo_kind", "upgrades", "ref"] {
        let Some(v) = e.get(key).filter(|v| !v.is_nil()) else { continue };
        if !s.is_empty() {
            s.push_str(", ");
        }
        match key {
            "upgrades" => {
                let n = text::split(&v.to_text(""), ',').len();
                s.push_str(&format!("{n} upgrade{}", if n == 1 { "" } else { "s" }));
            }
            "ref" => s.push_str(&format!("ref {}", v.to_text(""))),
            "ammo_kind" => s.push_str(&v.to_text("")),
            _ => match v.as_number() {
                Some(n) => s.push_str(&format!("{key} {}", look::num(n))),
                None => s.push_str(key),
            },
        }
    }
    s
}

/// The upgrades installed on the item, ticked from what its section can take.
fn upgrades(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    depth: u32,
    item_caps: &Caps,
    v: &mut Value,
) -> bool {
    let chosen: Vec<String> = text::split(&v.to_text(""), ',')
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let mut shown = chosen.join(", ");
    let pid = ui.make_persistent_id(("gz-ui-graph.upgrades", salt));
    let mut ch = false;
    let mut open = false;
    grid(ui, cx.role, (salt, "upgrades"), |g| {
        if g.row(
            "Upgrades",
            "The upgrades installed on the item, picked from what its section can take",
            PropFlags::NONE,
            depth,
            |ui| grid::text_field(ui, &mut shown).changed(),
        )
        .unwrap_or(false)
        {
            *v = if shown.trim().is_empty() { Value::Nil } else { Value::from(shown.as_str()) };
            ch = true;
        }
        open = g.trailing(Icon::More, "Pick the upgrades");
    });
    if open {
        cx.open_picker(pid);
    }
    if egui::Popup::is_id_open(&cx.ctx, pid) {
        let anchor = Rect::from_min_size(cx.pick.anchor, Vec2::ZERO);
        let mut toggled: Option<(String, bool)> = None;
        menu_popup(&cx.ctx, pid, anchor, ui.layer_id(), |ui| {
            if item_caps.upgrades.is_empty() {
                gz_ui::text::label(
                    ui,
                    if item_caps.known {
                        "This item has no upgrades"
                    } else {
                        "Link a game to list the upgrades"
                    },
                    Font::Body,
                    palette(ui.ctx()).text_disabled,
                    Clip::End,
                );
            }
            for id in &item_caps.upgrades {
                let mut on = chosen.iter().any(|c| c == id);
                ui.horizontal(|ui| {
                    if gz_ui::Check::new(&mut on, id).show(ui).changed() {
                        toggled = Some((id.clone(), on));
                    }
                });
            }
        });
        if let Some((id, on)) = toggled {
            // in the section's own order, an id at most once: the engine aborts on a duplicate
            let out: Vec<&str> = item_caps
                .upgrades
                .iter()
                .filter(|c| if **c == id { on } else { chosen.iter().any(|x| x == *c) })
                .map(String::as_str)
                .collect();
            *v = if out.is_empty() { Value::Nil } else { Value::from(out.join(", ")) };
            ch = true;
        }
    }
    ch
}

/// The state of one row, filtered by what its section can carry.
fn row_state(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, depth: u32, k: &Kind, e: &mut Value) -> bool {
    let mut ch = false;
    let sect = e.get_string("section", "");
    let item_caps = cx.data.caps.as_ref().map(|c| c.of(&sect)).unwrap_or_default();
    let count = e.get_number("count", 1.0) as i64;
    for (i, rp) in k.rows.iter().enumerate() {
        if rp.name == "section" || rp.name == "count" {
            continue;
        }
        // a ref names one object: offered only where it can mean something, kept when written
        if rp.name == "ref" && count > 1 && !e.has("ref") {
            continue;
        }
        let need = caps::cap_of_key(&rp.name);
        let stale = need != caps::ALL && item_caps.known && !item_caps.has(need);
        if stale && !e.has(&rp.name) {
            continue;
        }
        let mut val = e.get(&rp.name).cloned().unwrap_or_default();
        let help = if stale {
            format!("{} does not apply to {sect} - {}", rp.name, caps::why(need))
        } else {
            "Unset means full. condition: wear, 0 ruined. ammo: rounds loaded, or in one box. ref: one item only".to_owned()
        };
        let mut row = Row::new("", &help, depth);
        if stale {
            row = row.flags(PropFlags::INVALID);
        }
        let s = format!("{salt}/state{i}");
        let one = if rp.name == "ammo_kind" && !item_caps.ammo_class.is_empty() {
            // the rounds this weapon takes, not every item section in the game
            let mut cur = val.to_text("");
            row.label = "Cartridge".to_owned();
            let c = combo_row(ui, cx.role, &s, &row, &mut cur, &item_caps.ammo_class, Some("(first)"));
            if c {
                val = opt_str(&cur);
            }
            c
        } else if rp.name == "upgrades" {
            upgrades(ui, cx, &s, depth, &item_caps, &mut val)
        } else {
            let mut label = look::pretty(&rp.name);
            // an unset value means FULL, so that is what the field shows
            let full = item_caps.full_ammo();
            let mut shown = val.clone();
            if rp.name == "ammo" && val.is_nil() && full > 0 {
                shown = Value::Number(f64::from(full));
            }
            if rp.name == "ammo" && item_caps.known && full > 0 {
                label.push_str(&format!(" / {full}"));
            }
            row.label = label;
            let c = typed(ui, cx, &rp.ty, Some(rp), &s, &row, &mut shown);
            if c {
                val = shown;
                // typing the full value back is the same as saying nothing
                if rp.name == "ammo" && val.as_number().is_some_and(|n| n as i32 == full) {
                    val = Value::Nil;
                }
            }
            c
        };
        if one {
            if val.is_nil() {
                e.erase(&rp.name);
            } else {
                e.set(&rp.name, val);
            }
            ch = true;
        }
    }
    ch
}

/// The rows of "what to bring": a section with a count, or one concrete object.
#[allow(clippy::too_many_lines)]
pub fn item_list(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    objects_ok: bool,
    rows_of: Option<&Kind>,
) -> bool {
    let mut ch = false;
    let count_or_all = rows_of.and_then(|k| k.row_param("count")).is_some_and(|p| p.ty == "count_or_all");
    let with_state = rows_have_state(rows_of);
    let help = if objects_ok {
        "A row is either N items of a section, or one concrete object (ref/story id). Rows mix freely"
    } else if with_state {
        "A row is N items of a section with its own state. An ammo section counts in boxes"
    } else if count_or_all {
        "A row is N items of a section, or all of them. An ammo section counts in boxes"
    } else {
        "A row is N items of a section. An ammo section counts in boxes"
    };
    if !v.is_table() && !v.is_nil() {
        *v = Value::table();
        ch = true;
    }
    let n = v.arr().len();
    text_row(
        ui,
        cx.role,
        salt,
        &Row { help: help.to_owned(), ..row.clone() },
        &format!("{n} row{}", if n == 1 { "" } else { "s" }),
    );
    let d = row.depth + 1;
    let mut kill = None;
    for i in 0..n {
        let Some(mut e) = v.arr().get(i).cloned() else { continue };
        if !e.is_table() {
            e = Value::table();
        }
        let head = format!("Row {}", i + 1);
        let mut drop = false;
        let mut row_ch = false;
        if e.has("item") {
            let mut it = e.get("item").cloned().unwrap_or_default();
            if object_ref(
                ui,
                cx,
                &format!("{salt}/{i}/item"),
                &Row::new(&head, "One concrete object: a story id or a ref this graph created", d),
                &mut it,
                false,
            ) {
                e.set("item", it);
                row_ch = true;
            }
            ui.horizontal(|ui| {
                ui.add_space(12.0 * d as f32 + 6.0);
                if gz_ui::Button::new("Remove Row").kind(gz_ui::Kind::GhostDanger).show(ui).clicked() {
                    drop = true;
                }
            });
        } else {
            let mut sect = e.get_string("section", "");
            let pickers = cx.pickers();
            let unknown = !sect.is_empty()
                && pickers.is_some_and(|p| {
                    !p.entries(PickerType::Item).is_empty() && !p.known(PickerType::Item, &sect)
                });
            let pid = ui.make_persistent_id(("gz-ui-graph.pick", salt, i, "section"));
            let mut open = false;
            grid(ui, cx.role, (salt, i, "section"), |g| {
                if g.row(
                    &head,
                    "The item section: what to bring or make",
                    if unknown { PropFlags::INVALID } else { PropFlags::NONE },
                    d,
                    |ui| grid::text_field(ui, &mut sect).changed(),
                )
                .unwrap_or(false)
                {
                    row_ch = true;
                }
                if g.trailing(Icon::Delete, "Remove the row") {
                    drop = true;
                }
                open = g.trailing(Icon::More, "Pick from the list");
                if unknown {
                    g.message(Tone::Error, "Not an item section of the linked game");
                }
            });
            if open {
                cx.open_picker(pid);
            }
            if let Some(p) = cx.picker_popup(ui, pid, "item_section") {
                sect = p;
                row_ch = true;
            }
            if row_ch {
                e.set("section", sect);
            }
            if count_or_all {
                // a take row may say "all"; the default of 1 is not written
                let mut s = match e.get("count") {
                    None => "1".to_owned(),
                    Some(Value::Number(n)) => format!("{}", *n as i64),
                    Some(other) => other.to_text(""),
                };
                if text_row_edit(
                    ui,
                    cx.role,
                    &format!("{salt}/{i}/count"),
                    &Row::new("Count", "How many, or \"all\"", d + 1),
                    &mut s,
                ) {
                    let c = s.trim();
                    if c.is_empty() || c == "1" {
                        e.erase("count");
                    } else if c.bytes().all(|b| b.is_ascii_digit()) {
                        e.set("count", text::c_atof(c));
                    } else {
                        // "all", or a typo the validator names
                        e.set("count", c);
                    }
                    row_ch = true;
                }
            } else {
                let mut count = e.get_number("count", 1.0) as i32;
                let changed = grid(ui, cx.role, (salt, i, "count"), |g| {
                    g.row("Count", "How many", PropFlags::NONE, d + 1, |ui| {
                        grid::integer(ui, &mut count, 0.25, "").changed()
                    })
                    .unwrap_or(false)
                });
                if changed {
                    let count = count.clamp(1, 100_000);
                    if count == 1 {
                        e.erase("count");
                    } else {
                        e.set("count", f64::from(count));
                    }
                    row_ch = true;
                }
            }
            // the state of the row, folded away unless it already carries some
            if with_state && let Some(k) = rows_of {
                let sum = row_state_summary(&e);
                let key = ui.make_persistent_id(("gz-ui-graph.state_open", salt, i));
                let mut open_state = cx.ctx.data(|dd| dd.get_temp::<bool>(key)).unwrap_or(!sum.is_empty());
                let mut flip = false;
                grid(ui, cx.role, (salt, i, "state"), |g| {
                    g.row(
                        "State",
                        "The state every item of the row is made with; unset means full",
                        PropFlags::READ_ONLY,
                        d + 1,
                        |ui| grid::read_only(ui, if sum.is_empty() { "full" } else { &sum }, Clip::End),
                    );
                    flip = g.trailing(
                        if open_state { Icon::ChevronUp } else { Icon::ChevronDown },
                        if open_state { "Hide the state" } else { "Edit the state" },
                    );
                });
                if flip {
                    open_state = !open_state;
                    cx.ctx.data_mut(|dd| dd.insert_temp(key, open_state));
                }
                if open_state {
                    row_ch |= row_state(ui, cx, &format!("{salt}/{i}"), d + 2, k, &mut e);
                }
            }
        }
        if row_ch && let Some(slot) = v.table_mut().and_then(|t| t.arr.get_mut(i)) {
            *slot = e;
            ch = true;
        }
        if drop {
            kill = Some(i);
        }
    }
    if let Some(i) = kill
        && let Some(t) = v.table_mut()
    {
        t.arr.remove(i);
        if t.is_empty() {
            *v = Value::Nil;
        }
        ch = true;
    }
    let mut add_section = false;
    let mut add_object = false;
    ui.horizontal(|ui| {
        ui.add_space(12.0 * d as f32 + 6.0);
        add_section = gz_ui::Button::new("Add Section Row").kind(gz_ui::Kind::Ghost).show(ui).clicked();
        if objects_ok {
            add_object = gz_ui::Button::new("Add Object").kind(gz_ui::Kind::Ghost).show(ui).clicked();
        }
    });
    if add_section {
        let mut e = Value::table();
        e.set("section", "");
        v.push(e);
        ch = true;
    }
    if add_object {
        let mut r = Value::table();
        r.set("story", "");
        let mut e = Value::table();
        e.set("item", r);
        v.push(e);
        ch = true;
    }
    ch
}

/// The list of rows of an item action: the one-line spelling shown as a one-row list until the
/// first edit rewrites it as `items`.
fn item_rows_params(ui: &mut Ui, cx: &mut ParamCx<'_>, k: &Kind, params: &mut Value, depth: u32) -> bool {
    let mut rows = match params.get("items") {
        Some(it) => it.clone(),
        None if params.has("section") => {
            let mut e = Value::table();
            e.set("section", params.get_string("section", ""));
            if let Some(c) = params.get("count")
                && c.as_number() != Some(1.0)
            {
                e.set("count", c.clone());
            }
            for rp in &k.rows {
                if rp.name == "section" || rp.name == "count" {
                    continue;
                }
                if let Some(sv) = params.get(&rp.name) {
                    e.set(&rp.name, sv.clone());
                }
            }
            Value::from(vec![e])
        }
        None => Value::Nil,
    };
    if item_list(
        ui,
        cx,
        "rows_items",
        &Row::new("Items", "", depth).flags(PropFlags::REQUIRED),
        &mut rows,
        false,
        Some(k),
    ) {
        params.erase("section");
        params.erase("count");
        // whatever the rows carry moved into them
        for rp in &k.rows {
            params.erase(&rp.name);
        }
        if rows.is_empty() {
            params.erase("items");
        } else {
            params.set("items", rows);
        }
        return true;
    }
    false
}

/// item.spawn's one destination.
fn item_spawn_where(ui: &mut Ui, cx: &mut ParamCx<'_>, params: &mut Value, depth: u32) -> bool {
    let mut v = params.get("where").cloned().unwrap_or_default();
    let row = Row::new("Where", "Where they land: on the ground, inside a container or NPC, in a stash or a corpse, or in a fresh corpse", depth).flags(PropFlags::REQUIRED);
    if loot_spawn(ui, cx, "spawn_where", &row, &mut v) {
        if v.is_empty() {
            params.erase("where");
        } else {
            params.set("where", v);
        }
        return true;
    }
    false
}

/// The whole fetch node as one form: the rows, then the modifiers.
fn fetch_params(ui: &mut Ui, cx: &mut ParamCx<'_>, k: &Kind, params: &mut Value, depth: u32) -> bool {
    let mut ch = false;
    let mut rows = match params.get("items") {
        Some(it) => it.clone(),
        None if params.has("section") => {
            let mut e = Value::table();
            e.set("section", params.get_string("section", ""));
            let n = params.get_number("count", 1.0);
            if n > 1.0 {
                e.set("count", n);
            }
            Value::from(vec![e])
        }
        None => Value::Nil,
    };
    if item_list(ui, cx, "fetch_items", &Row::new("Items", "", depth), &mut rows, true, None) {
        ch = true;
        params.erase("section");
        params.erase("count");
        if rows.is_empty() {
            params.erase("items");
        } else {
            params.set("items", rows);
        }
    }
    let mut from = params.get("from").cloned().unwrap_or_default();
    if object_ref(
        ui,
        cx,
        "fetch_from",
        &Row::new(
            "From container",
            "Optional: credit only items taken from this container. An identical item from elsewhere does not count",
            depth,
        ),
        &mut from,
        false,
    ) {
        ch = true;
        if from.is_empty() {
            params.erase("from");
        } else {
            params.set("from", from);
        }
    }
    let mut spawn = params.get("spawn").cloned().unwrap_or_default();
    if loot_spawn(
        ui,
        cx,
        "fetch_spawn",
        &Row::new(
            "Spawn loot",
            "Optional: the node creates the items on entry and credits only those instances",
            depth,
        ),
        &mut spawn,
    ) {
        ch = true;
        if spawn.is_empty() {
            params.erase("spawn");
        } else {
            params.set("spawn", spawn);
        }
    }
    // the marker binding draws through the same generic rows as kill and reach
    for (i, name) in ["spot_task", "spot_objective", "spot_done"].iter().enumerate() {
        if let Some(sp) = k.param(name) {
            ui.push_id(2000 + i, |ui| ch |= draw_param(ui, cx, sp, params, &k.id, depth));
        }
    }
    ch
}

/// WHERE the node puts the loot it creates: one mode at a time.
#[allow(clippy::too_many_lines)]
pub fn loot_spawn(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let list = modes(&["place", "into", "stash", "npc", "corpse", "new_corpse"]);
    let mut mode = mode_of(v, &list);
    let mut ch = false;
    let mut m = mode.clone();
    if mode_row(ui, cx.role, salt, row, &mut m, &list) {
        ch = true;
        *v = match m.as_str() {
            "" => Value::Nil,
            "place" => {
                let mut t = Value::table();
                t.set("place", fresh_pos(&cx.data.level, None));
                t
            }
            "new_corpse" => {
                let mut nc = Value::table();
                nc.set("section", "");
                nc.set("place", fresh_pos(&cx.data.level, None));
                let mut t = Value::table();
                t.set("new_corpse", nc);
                t
            }
            "stash" | "corpse" => {
                let mut t = Value::table();
                t.set(&m, Value::table());
                t
            }
            other => {
                let mut r = Value::table();
                r.set("story", "");
                let mut t = Value::table();
                t.set(other, r);
                t
            }
        };
        mode = m;
    }
    let d = row.depth + 1;
    match mode.as_str() {
        "place" => {
            let mut pl = v.get("place").cloned().unwrap_or_default();
            if place(
                ui,
                cx,
                &format!("{salt}/place"),
                &Row::new("Place", "Drops the loot on the ground at the place", d),
                &mut pl,
            ) {
                v.set("place", pl);
                ch = true;
            }
        }
        "into" => {
            let mut r = v.get("into").cloned().unwrap_or_default();
            if object_ref(
                ui,
                cx,
                &format!("{salt}/into"),
                &Row::new(
                    "Into object",
                    "A specific container: a story id (ALife\\Story ID (script) on the object) or a ref created by this graph",
                    d,
                ),
                &mut r,
                false,
            ) {
                v.set("into", r);
                ch = true;
            }
        }
        "npc" => {
            let mut r = v.get("npc").cloned().unwrap_or_default();
            if npc_ref(
                ui,
                cx,
                &format!("{salt}/npc"),
                &Row::new(
                    "NPC",
                    "Puts the loot into a live NPC inventory. profile/community match the first online one",
                    d,
                ),
                &mut r,
                false,
                false,
            ) {
                v.set("npc", r);
                ch = true;
            }
        }
        "stash" | "corpse" => {
            let mut sel = v.get(&mode).cloned().filter(Value::is_table).unwrap_or_else(Value::table);
            let mut sel_ch = false;
            let mut pick = sel.get_string("pick", "");
            if combo_row(
                ui,
                cx.role,
                &format!("{salt}/pick"),
                &Row::new("Pick", "Which of the matches gets the loot", d),
                &mut pick,
                &modes(&["random", "nearest", "farthest"]),
                Some("(default)"),
            ) {
                if pick.is_empty() {
                    sel.erase("pick");
                } else {
                    sel.set("pick", pick);
                }
                sel_ch = true;
            }
            if mode == "stash" {
                let mut st = sel.get_string("state", "");
                if combo_row(
                    ui,
                    cx.role,
                    &format!("{salt}/state"),
                    &Row::new("State", "Full: has anything inside. Empty: holds nothing", d),
                    &mut st,
                    &modes(&["any", "full", "empty"]),
                    Some("(any)"),
                ) {
                    if st.is_empty() {
                        sel.erase("state");
                    } else {
                        sel.set("state", st);
                    }
                    sel_ch = true;
                }
            }
            let mut within = sel.get_number("within", 0.0) as f32;
            if number_row(
                ui,
                cx.role,
                &format!("{salt}/within"),
                &Row::new("Within, m", "0 = unlimited. Nothing within it: the nearest match is used", d),
                &mut within,
                1.0,
                "m",
            ) {
                if within <= 0.0 {
                    sel.erase("within");
                } else {
                    sel.set("within", f64::from(within));
                }
                sel_ch = true;
            }
            let mut beyond = sel.get_number("beyond", 0.0) as f32;
            if number_row(
                ui,
                cx.role,
                &format!("{salt}/beyond"),
                &Row::new("Beyond, m", "0 = unlimited. Nothing beyond it: the farthest match is used", d),
                &mut beyond,
                1.0,
                "m",
            ) {
                if beyond <= 0.0 {
                    sel.erase("beyond");
                } else {
                    sel.set("beyond", f64::from(beyond));
                }
                sel_ch = true;
            }
            if mode == "stash" {
                let mut scatter = sel.get_bool("scatter", false);
                if check_row(
                    ui,
                    cx.role,
                    &format!("{salt}/scatter"),
                    &Row::new(
                        "Scatter",
                        "Each item goes to its own matching stash, otherwise all share one",
                        d,
                    ),
                    &mut scatter,
                ) {
                    if scatter {
                        sel.set("scatter", true);
                    } else {
                        sel.erase("scatter");
                    }
                    sel_ch = true;
                }
            }
            if sel_ch {
                v.set(&mode, sel);
                ch = true;
            }
        }
        "new_corpse" => {
            let mut nc = v.get("new_corpse").cloned().filter(Value::is_table).unwrap_or_else(Value::table);
            let mut nc_ch = false;
            let mut sect = nc.get_string("section", "");
            if picked(
                ui,
                cx,
                &format!("{salt}/ncs"),
                &Row::new("Corpse section", "The corpse's spawn section", d),
                "spawn_section",
                &mut sect,
            ) {
                nc.set("section", sect);
                nc_ch = true;
            }
            let mut pl = nc.get("place").cloned().unwrap_or_default();
            if place(
                ui,
                cx,
                &format!("{salt}/ncp"),
                &Row::new("Place", "Where the fresh corpse drops. The loot goes into its inventory", d),
                &mut pl,
            ) {
                nc.set("place", pl);
                nc_ch = true;
            }
            if nc_ch {
                v.set("new_corpse", nc);
                ch = true;
            }
        }
        _ => {}
    }
    ch
}

/// The cases of a node: each one a pin.
pub fn cases(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value, weights: bool) -> bool {
    if !v.is_table() {
        *v = Value::table();
    }
    let mut ch = false;
    let n = v.arr().len();
    let more = if weights {
        "Each case is a pin; the weights decide how often each one fires"
    } else {
        "Each case is a pin; the first case whose conditions hold fires"
    };
    text_row(
        ui,
        cx.role,
        salt,
        &row.clone().more_help(more),
        &format!("{n} case{}", if n == 1 { "" } else { "s" }),
    );
    let d = row.depth + 1;
    let mut drop = None;
    for i in 0..n {
        let Some(mut c) = v.arr().get(i).cloned() else { continue };
        if !c.is_table() {
            c = Value::table();
        }
        let mut c_ch = false;
        let mut name = c.get_string("name", "");
        grid(ui, cx.role, (salt, i, "name"), |g| {
            if g.row(
                &format!("Case {}", i + 1),
                "The case's name: the pin it leaves the node by",
                PropFlags::NONE,
                d,
                |ui| grid::text_field(ui, &mut name).changed(),
            )
            .unwrap_or(false)
            {
                c_ch = true;
            }
            if g.trailing(Icon::Delete, "Remove the case") {
                drop = Some(i);
            }
        });
        if c_ch {
            c.set("name", name);
        }
        if weights {
            let mut w = c.get_number("weight", 1.0) as f32;
            if number_row(
                ui,
                cx.role,
                &format!("{salt}/{i}/weight"),
                &Row::new("Weight", "How often this case fires against the others", d + 1),
                &mut w,
                0.1,
                "",
            ) {
                c.set("weight", f64::from(w));
                c_ch = true;
            }
        } else {
            let mut cl = c.get("cond").cloned().unwrap_or_else(Value::table);
            if cond_value(ui, cx, &format!("{salt}/{i}/cond"), &Row::new("cond", "", d + 1), &mut cl, 1) {
                c.set("cond", cl);
                c_ch = true;
            }
        }
        if c_ch && let Some(slot) = v.table_mut().and_then(|t| t.arr.get_mut(i)) {
            *slot = c;
            ch = true;
        }
    }
    if let Some(i) = drop
        && let Some(t) = v.table_mut()
    {
        t.arr.remove(i);
        ch = true;
    }
    if add_button(ui, "Add Case", d) {
        let mut c = Value::table();
        c.set("name", format!("case{}", n + 1));
        if weights {
            c.set("weight", 1.0);
        } else {
            c.set("cond", Value::table());
        }
        v.push(c);
        ch = true;
    }
    ch
}

/// A condition list inside parameters: generic values edited as typed conditions.
pub fn cond_value(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row: &Row,
    v: &mut Value,
    depth: u32,
) -> bool {
    let mut conds: Vec<Cond> = v.arr().iter().filter_map(|c| asset::cond_from_value(c).ok()).collect();
    if !cond_list(ui, cx, salt, row.depth, &mut conds, depth) {
        return false;
    }
    *v = Value::from(conds.iter().map(asset::cond_to_value).collect::<Vec<_>>());
    true
}

/// The conditions of a node or a case: "If" then "And", each with its kind and its parameters.
pub fn cond_list(
    ui: &mut Ui,
    cx: &mut ParamCx<'_>,
    salt: &str,
    row_depth: u32,
    conds: &mut Vec<Cond>,
    nesting: u32,
) -> bool {
    let mut ch = false;
    let mut drop = None;
    for (i, c) in conds.iter_mut().enumerate() {
        let desc = look::kind_desc(cx.cat, &c.kind, cx.behaviour);
        let help = if desc.is_empty() { "A condition: all of them have to hold".to_owned() } else { desc };
        let mut kind = c.kind.to_string();
        let mut negate = c.negate;
        let mut kind_ch = false;
        let cat = cx.cat;
        let behaviour = cx.behaviour;
        grid(ui, cx.role, (salt, i, "cond"), |g| {
            g.row(if i == 0 { "If" } else { "And" }, &help, PropFlags::NONE, row_depth, |ui| {
                if gz_ui::Switch::new(&mut negate, "not")
                    .tip("Holds when the condition does not")
                    .show(ui)
                    .changed()
                {
                    ch = true;
                }
                ui.add_space(4.0);
                if look::kind_combo(ui, "kind", cat, USE_COND, behaviour, &mut kind) {
                    kind_ch = true;
                }
            });
            if g.trailing(Icon::Delete, "Remove the condition") {
                drop = Some(i);
            }
        });
        c.negate = negate;
        if kind_ch && kind != c.kind {
            c.kind = SmolStr::new(&kind);
            c.params = Value::table();
            ch = true;
        }
        if nesting < 4
            && let Some(k) = cx.cat.find(&c.kind)
            && !k.params.is_empty()
        {
            let k = k.clone();
            ui.push_id((salt, i, "params"), |ui| ch |= draw_params(ui, cx, &k, &mut c.params, row_depth + 1));
        }
    }
    if let Some(i) = drop {
        conds.remove(i);
        ch = true;
    }
    if add_button(ui, "Add Condition", row_depth) {
        conds.push(Cond { kind: SmolStr::new("has_item"), params: Value::table(), negate: false });
        ch = true;
    }
    ch
}

/// Lua in a row: its first line; the body is written in a dialog with a syntax check.
pub fn lua(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let target = format!("{:?}/{salt}/lua", ui.scope_id());
    let mut code = v.to_text("");
    let mut ch = false;
    if let Some(applied) = cx.text_edit.take(&target) {
        code = applied;
        *v = Value::from(code.clone());
        ch = true;
    }
    let mut edit = false;
    grid(ui, cx.role, salt, |g| {
        g.row(&row.label, &row.help, row.flags, row.depth, |ui| {
            grid::read_only(
                ui,
                &if code.is_empty() { "(empty)".to_owned() } else { first_line(&code) },
                Clip::End,
            )
        });
        edit = g.trailing(Icon::Rename, "Edit the Lua");
    });
    if edit {
        cx.text_edit.open_for(&target, &row.label, &code, true);
    }
    ch
}

/// A bool, a number or a string typed by its content.
pub fn scalar_value(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let mut s = match &*v {
        Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
        Value::Number(n) => look::num(*n),
        other => other.to_text(""),
    };
    if !text_row_edit(ui, cx.role, salt, row, &mut s) {
        return false;
    }
    let t = s.trim();
    *v = match t {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => match gz_luadata::Value::from(t).to_number(f64::NAN) {
            n if !t.is_empty() && !n.is_nan() => Value::Number(n),
            _ => Value::from(s.as_str()),
        },
    };
    true
}

/// A value the catalog does not know, kept and edited as Lua.
pub fn raw(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, v: &mut Value) -> bool {
    let mut s = gz_luadata::write_value(v, 0);
    if !text_row_edit(ui, cx.role, salt, row, &mut s) {
        return false;
    }
    match gz_luadata::parse(&format!("return {s}")) {
        Ok(nv) => {
            *v = nv;
            true
        }
        Err(_) => false,
    }
}

/// A combo of the variables the graph declares.
pub fn var_name_combo(ui: &mut Ui, cx: &mut ParamCx<'_>, salt: &str, row: &Row, name: &mut String) -> bool {
    let names: Vec<String> = cx.vars.iter().map(|v| v.name.clone()).collect();
    if names.is_empty() && name.is_empty() {
        text_row(ui, cx.role, salt, row, "No variables - add one in the Graph panel");
        return false;
    }
    // a name no longer declared stays selectable (combo_field keeps it)
    combo_row(ui, cx.role, salt, row, name, &names, Some("(unset)"))
}

/// The help footer shows a kind's description on the kind row; `Id` for a popup a caller opens.
#[must_use]
pub fn popup_id(ui: &Ui, salt: &str) -> Id {
    ui.make_persistent_id(("gz-ui-graph.popup", salt))
}
