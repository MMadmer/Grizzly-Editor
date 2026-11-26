//! The two side panels of a graph tab: the Graph panel (the document's own
//! settings in one scroll: its header, its tasks or roles, its variables, then the module's
//! globals - each a section with a + on its header) and the Details panel (the selected node, its selected action under a splitter, the
//! help footer). Edits are staged on a copy while a widget is worked and committed as one
//! document command once it is let go of.

use std::sync::Arc;

use egui::{Align, Rect, ScrollArea, Ui, UiBuilder, pos2, vec2};
use gz_editor::Editor;
use gz_luadata::Value;
use gz_nq::asset::{self, Graph, Node, Objective, Role, Task, Var};
use gz_nq::catalog::{USE_EXTRA, USE_MAIN, USE_TRIGGER};
use gz_nq::doc::DocOp;
use gz_nq::{Catalog, text};
use gz_ui::grid::{self, PropFlags};
use gz_ui::{Clip, Font, Icon, NavRowSpec, TipExt, Tone, Tx, nav_row, palette, tr};
use smol_str::SmolStr;

use crate::canvas::{Op, ViewMirror};
use crate::derived::DocData;
use crate::local;
use crate::look;
use crate::params::{self, ParamCx, PickState, Row, TextEdit};
use gz_ui::section::{self, Section};

pub const ACTION_SPLIT: &str = "graph.action_split";
const ROLE_HEADER: &str = "graph_header";
const ROLE_TASKS: &str = "graph_tasks";
const ROLE_ROLES: &str = "graph_roles";
const ROLE_VARS: &str = "graph_vars";
const ROLE_GLOBALS: &str = "graph_globals";
const ROLE_NODE: &str = "graph_node";
const ROLE_ACTION: &str = "graph_action";

/// The document's own settings as the Graph panel stages them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub id: String,
    pub title: Value,
    pub activation: String,
    pub vars: Vec<Var>,
    pub tasks: Vec<Task>,
    pub roles: Vec<Role>,
}

impl Header {
    #[must_use]
    pub fn of(g: &Graph) -> Self {
        Self {
            id: g.id.clone(),
            title: g.title.clone(),
            activation: g.activation.clone(),
            vars: g.vars.clone(),
            tasks: g.tasks.iter().cloned().collect(),
            roles: g.roles.clone(),
        }
    }
}

/// A task as the writer emits it, for `set_quest`.
fn task_value(t: &Task) -> Value {
    let mut tv = Value::table();
    if !t.title.is_nil() {
        tv.put("title", t.title.clone());
    }
    if !t.descr.is_nil() {
        tv.put("descr", t.descr.clone());
    }
    tv.put("type", if t.ty.is_empty() { "additional" } else { t.ty.as_str() });
    if !t.target.is_nil() {
        tv.put("target", t.target.clone());
    }
    if !t.icon.is_empty() {
        tv.put("icon", t.icon.as_str());
    }
    if !t.auto_complete && !t.objectives.is_empty() {
        tv.put("auto_complete", false);
    }
    if !t.objectives.is_empty() {
        let objs: Vec<Value> = t
            .objectives
            .iter()
            .map(|o| {
                let mut ov = Value::table();
                ov.put("id", o.id.as_str());
                if !o.title.is_nil() {
                    ov.put("title", o.title.clone());
                }
                if !o.descr.is_nil() {
                    ov.put("descr", o.descr.clone());
                }
                if !o.target.is_nil() {
                    ov.put("target", o.target.clone());
                }
                if !o.visible {
                    ov.put("visible", false);
                }
                ov
            })
            .collect();
        tv.put("objectives", Value::from(objs));
    }
    tv
}

/// A global's default as written in mod.ltx, as the value it is.
#[must_use]
pub fn global_value(text_: &str) -> Value {
    match text_ {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            let n = Value::from(text_).to_number(f64::NAN);
            if !text_.is_empty() && !n.is_nan() {
                Value::Number(n)
            } else if text_.len() >= 2 && text_.starts_with('"') && text_.ends_with('"') {
                Value::from(text_.get(1..text_.len() - 1).unwrap_or(""))
            } else {
                Value::from(text_)
            }
        }
    }
}

/// A global's value as mod.ltx keeps it: a string always quoted.
#[must_use]
pub fn global_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
        Value::Number(n) => look::num(*n),
        other => format!("\"{}\"", other.to_text("")),
    }
}

/// Where a pane's scroll stood on the last frame: `[y, max]`.
pub type Scroll = [f32; 2];

/// The panels of one tab.
#[derive(Default)]
pub struct Inspector {
    /// The tab's file: what scopes the panels' fold states.
    scope: String,
    node_id: SmolStr,
    node: Node,
    node_edit: u64,
    node_dirty: bool,
    header: Header,
    header_edit: Option<u64>,
    header_dirty: bool,
    id_buf: String,
    id_buf_of: SmolStr,
    /// The UI pass the ID field last had the focus in: a rename left by a click elsewhere is
    /// committed only right after it was being typed.
    id_focus_pass: u64,
    var_sel: usize,
    var_name_buf: String,
    /// The variable whose name `var_name_buf` holds.
    var_name_of: String,
    globals: Vec<(String, Value)>,
    globals_rev: Option<gz_base::Rev>,
    globals_dirty: bool,
    global_sel: usize,
    global_name_buf: String,
    reveal_key: String,
    reveal_frames: u8,
    /// When the action shown last changed: a pane that changes its height soon after shows it
    /// again, one that changes later (a drawer, the window) leaves the author's scroll alone.
    reveal_at: f64,
    reveal_open: bool,
    /// The node pane's height last frame.
    reveal_pane_h: f32,
    pub focus_rename: bool,
    pub focus_action: bool,
    pub pick: PickState,
    pub text_edit: TextEdit,
    /// The panes as the last frame drew them (pt of the screen).
    pub node_pane: Option<Rect>,
    pub action_pane: Option<Rect>,
    pub graph_pane: Option<Rect>,
    pub vars_pane: Option<Rect>,
    pub node_scroll: Scroll,
    pub action_scroll: Scroll,
    pub graph_scroll: Scroll,
    pub vars_scroll: Scroll,
    pub action_share: f32,
    pub vars_share: f32,
    details_column: f32,
    pending_action: Option<f32>,
    /// graph.view scroll=: taken by the pane it names.
    pub scroll: Option<String>,
    /// A pane to scroll to its end on the next frame, once its content is measured.
    bottom_next: Option<&'static str>,
    /// Where the Graph panel's tasks and their steps start in its content (pt), last frame.
    anchors: Vec<(String, f32)>,
    /// A task whose Rename or References the panel asked for.
    pub rename_task: Option<String>,
    pub refs_task: Option<String>,
    /// The Add Action button's request: (node, exit, where).
    pub add_action: Option<(String, bool, egui::Pos2)>,
    /// What went wrong, for the status line.
    pub notes: Vec<String>,
}

/// What a frame of the panels reads.
pub struct PanelCx<'a> {
    pub editor: &'a Arc<Editor>,
    pub cat: &'a Catalog,
    pub graph: &'a Graph,
    pub edit: u64,
    pub data: &'a DocData,
    pub view: &'a mut ViewMirror,
    pub ops: &'a mut Vec<Op>,
    /// Commands of the document are still on their way: a whole-graph replace must wait.
    pub pending: bool,
}

fn split_slot(slot: &str) -> Option<(bool, usize)> {
    let (s, i) = slot.split_once(':')?;
    let exit = match s {
        "enter" => false,
        "exit" => true,
        _ => return None,
    };
    Some((exit, i.parse().ok()?))
}

fn op(fields: &[(&str, Value)]) -> Value {
    let mut t = Value::table();
    for (k, v) in fields {
        t.set(k, v.clone());
    }
    t
}

/// A variable renamed in a value: `var` / `var.set` / `var.add` tables nested anywhere.
fn rename_var_in(v: &mut Value, from: &str, to: &str) {
    let Value::Table(t) = v else { return };
    let kind = t.get("kind").and_then(Value::as_str).unwrap_or("").to_owned();
    if matches!(kind.as_str(), "var" | "var.set" | "var.add")
        && let Some(nm) = t.get_mut("name")
        && nm.as_str() == Some(from)
    {
        *nm = Value::from(to);
    }
    for (_, x) in &mut t.fields {
        rename_var_in(x, from, to);
    }
    for x in &mut t.arr {
        rename_var_in(x, from, to);
    }
}

/// A fold header of the panels: the kit's section, with one action on its header. `title` is
/// the editor's word, or data (a task's id) said with `Tx::raw`.
fn fold<'a>(
    ui: &mut Ui,
    key: &'a str,
    title: impl Into<Tx<'a>>,
    count: Option<&'a str>,
    open: bool,
    action: Option<(Icon, &'static str)>,
) -> (bool, bool) {
    let mut sec = Section::new(key, title).default_open(open);
    if let Some(c) = count {
        sec = sec.count(Tx::raw(c));
    }
    if let Some((icon, tip)) = action {
        sec = sec.action(icon, tip);
    }
    let r = sec.show(ui);
    (r.open, r.action == Some(0))
}

impl Inspector {
    #[must_use]
    pub fn new(scope: &str) -> Self {
        Self { action_share: 0.5, vars_share: 0.38, scope: scope.to_owned(), ..Self::default() }
    }

    /// A section's key: what is folded stays folded in every graph.
    fn key(&self, k: &str) -> String {
        let _ = &self.scope;
        format!("graph.{k}")
    }

    /// Opens or folds every role's More (`graph.view roles_more=`).
    pub fn set_roles_more(&self, ctx: &egui::Context, open: bool) {
        for i in 0..self.header.roles.len() {
            section::set_open(ctx, &self.key(&format!("role{i}/more")), open);
        }
    }

    /// An action is open under the node.
    #[must_use]
    pub fn has_action(view: &ViewMirror) -> bool {
        view.selection.len() == 1 && split_slot(&view.slot).is_some()
    }

    /// The Details panel's split for a caller without a mouse, as a share of its column.
    pub fn set_split(&mut self, ctx: &egui::Context, action: Option<f32>) {
        if let Some(f) = action {
            let f = f.clamp(0.1, 0.9);
            if self.details_column > 0.0 {
                gz_ui::layout::set(ctx, ACTION_SPLIT, f * self.details_column);
            } else {
                self.pending_action = Some(f);
            }
            self.action_share = f;
        }
    }

    // ---- staging ------------------------------------------------------------------------------

    fn sync_node(&mut self, cx: &mut PanelCx<'_>, pass: u64) {
        let sel = match cx.view.selection.as_slice() {
            [one] => one.clone(),
            _ => SmolStr::default(),
        };
        // a copy refilled while our own commands are on their way would show the old text
        if sel != self.node_id || (self.node_edit != cx.edit && !cx.pending) {
            // a pending edit reaches the document before the copy is refilled
            if self.node_dirty {
                self.commit_node(cx);
            }
            // an id typed and left by a click elsewhere (the canvas is drawn first, so the
            // selection moved before the field could lose its focus) is a rename all the same
            let typed = self.id_buf.trim().to_owned();
            if sel != self.node_id
                && !self.node_id.is_empty()
                && self.id_buf_of == self.node_id
                && pass <= self.id_focus_pass + 1
                && !typed.is_empty()
                && typed != self.node_id.as_str()
                && cx.graph.find_node(&self.node_id).is_some()
            {
                cx.ops
                    .push(Op::Doc(DocOp::RenameNode { id: self.node_id.to_string(), new_id: typed.clone() }));
                // a selection that still holds the node holds it under its new id
                for s in &mut cx.view.selection {
                    if *s == self.node_id {
                        *s = SmolStr::new(&typed);
                    }
                }
            }
            self.id_buf_of = SmolStr::default();
            self.node_id = sel.clone();
            self.node_edit = cx.edit;
            self.node_dirty = false;
            self.node = cx.graph.find_node(&sel).cloned().unwrap_or_default();
        }
    }

    fn commit_node(&mut self, cx: &mut PanelCx<'_>) {
        if !self.node_dirty || self.node_id.is_empty() {
            return;
        }
        self.node_dirty = false;
        let Some(cur) = cx.graph.find_node(&self.node_id) else { return };
        let n = &self.node;
        if asset::node_to_value(cur)
            == asset::node_to_value(&Node { pos: cur.pos, has_pos: cur.has_pos, ..n.clone() })
        {
            return;
        }
        // the canvas owns the coordinates: the panel never writes a stale copy over a drag
        let mut patch = Value::table();
        patch.set("kind", n.kind.as_str());
        if n.once_set {
            patch.set("once", n.once);
        } else {
            patch.set("once", "default");
        }
        patch.set("params", if n.params.is_empty() { Value::table() } else { n.params.clone() });
        patch.set("comment", n.comment.as_str());
        patch.set("cond", Value::from(n.cond.iter().map(asset::cond_to_value).collect::<Vec<_>>()));
        patch.set("on_enter", Value::from(n.on_enter.iter().map(asset::action_to_value).collect::<Vec<_>>()));
        patch.set("on_exit", Value::from(n.on_exit.iter().map(asset::action_to_value).collect::<Vec<_>>()));
        let mut out = Value::table();
        for p in n.out.iter().filter(|p| !p.targets.is_empty()) {
            match p.targets.as_slice() {
                [one] => out.set(&p.name, one.as_str()),
                many => out.set(
                    &p.name,
                    Value::from(many.iter().map(|t| Value::from(t.as_str())).collect::<Vec<_>>()),
                ),
            };
        }
        patch.set("out", out);
        cx.ops.push(Op::Doc(DocOp::Apply(op(&[
            ("op", Value::from("set_node")),
            ("id", Value::from(self.node_id.as_str())),
            ("patch", patch),
        ]))));
    }

    fn sync_header(&mut self, cx: &mut PanelCx<'_>) {
        if self.header_edit != Some(cx.edit) && !self.header_dirty && !cx.pending {
            self.header = Header::of(cx.graph);
            self.header_edit = Some(cx.edit);
        }
    }

    fn commit_header(&mut self, cx: &mut PanelCx<'_>) {
        if !self.header_dirty {
            return;
        }
        let h = &self.header;
        if h.roles != cx.graph.roles {
            // set_quest knows no roles: the whole graph goes, once nothing else is on its way
            if cx.pending {
                return;
            }
            let mut g = cx.graph.clone();
            g.id.clone_from(&h.id);
            g.title = h.title.clone();
            g.activation.clone_from(&h.activation);
            g.vars.clone_from(&h.vars);
            g.tasks = h.tasks.iter().cloned().collect();
            g.roles.clone_from(&h.roles);
            cx.ops.push(Op::Doc(DocOp::Replace(g)));
        } else {
            let mut patch = Value::table();
            patch.put("id", h.id.as_str());
            patch.put("title", h.title.clone());
            if !cx.graph.is_behaviour() {
                patch.put("activation", h.activation.as_str());
            }
            let mut vars = Value::table();
            for v in &h.vars {
                vars.put(&v.name, v.value.clone());
            }
            patch.put("vars", vars);
            let mut tasks = Value::table();
            for t in &h.tasks {
                tasks.put(&t.id, task_value(t));
            }
            patch.put("tasks", tasks);
            cx.ops.push(Op::Doc(DocOp::Apply(op(&[("op", Value::from("set_quest")), ("patch", patch)]))));
        }
        self.header_dirty = false;
        self.header_edit = None;
    }

    /// Staged edits reach the document once no widget is being worked.
    pub fn commit_idle(&mut self, ctx: &egui::Context, cx: &mut PanelCx<'_>) {
        if ctx.egui_wants_keyboard_input() || ctx.dragged_id().is_some() {
            return;
        }
        self.commit_node(cx);
        self.commit_header(cx);
        if self.globals_dirty {
            self.save_globals(cx.editor);
        }
    }

    /// Everything staged goes now (a save, a close).
    pub fn flush(&mut self, cx: &mut PanelCx<'_>) {
        self.commit_node(cx);
        self.commit_header(cx);
        if self.globals_dirty {
            self.save_globals(cx.editor);
        }
    }

    fn param_cx<'s>(
        &'s mut self,
        ui: &Ui,
        cx: &'s PanelCx<'_>,
        role: &'static str,
        header_ctx: bool,
    ) -> ParamCx<'s> {
        ParamCx {
            ctx: ui.ctx().clone(),
            cat: cx.cat,
            behaviour: cx.graph.is_behaviour(),
            data: cx.data,
            graph: cx.graph,
            vars: if header_ctx { &[] } else { &self.header.vars },
            tasks: if header_ctx { &[] } else { &self.header.tasks },
            role,
            pick: &mut self.pick,
            text_edit: &mut self.text_edit,
            siblings: None,
            camera: gz_editor::ViewCamera::of(cx.editor),
        }
    }

    // ---- the Graph panel ----------------------------------------------------------------------

    /// The Graph panel into `rect`: the header, the tasks or roles, the variables and the
    /// module's globals in one scroll, each section with a + on its header.
    pub fn draw_graph_panel(&mut self, ui: &mut Ui, rect: Rect, cx: &mut PanelCx<'_>) {
        self.sync_header(cx);
        let ctx = ui.ctx().clone();
        let mut child = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.graph_panel").max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = 4.0;
        self.graph_pane = Some(rect);
        if self.scroll.as_deref() == Some("variables") {
            section::set_open(&ctx, &self.key("vars"), true);
        }
        let mine = cx.view.selection.len() != 1;
        let mut sa = ScrollArea::vertical().id_salt("gz-ui-graph.graph_scroll").auto_shrink([false, false]);
        if self.bottom_next == Some("graph") {
            sa = sa.vertical_scroll_offset(f32::MAX / 4.0);
            self.bottom_next = None;
        }
        let req = self.scroll.clone();
        let mut taken = false;
        match req.as_deref() {
            Some("top") if mine => {
                sa = sa.vertical_scroll_offset(0.0);
                taken = true;
            }
            Some(w) if w == "tasks" || w == "variables" || w.starts_with("task:") => {
                if let Some((_, y)) = self.anchors.iter().find(|(k, _)| k == w) {
                    sa = sa.vertical_scroll_offset(*y);
                    taken = true;
                } else if w.starts_with("task:") {
                    // a task's header opens on the way
                    let id = w.trim_start_matches("task:");
                    let id = id.split('/').next().unwrap_or(id);
                    section::set_open(&ctx, &self.key(&format!("task:{id}")), true);
                }
            }
            _ => {}
        }
        let mut anchors = Vec::new();
        let mut vars_span = (0.0, 0.0);
        let out = sa.show(&mut child, |ui| {
            let origin = ui.max_rect().top();
            self.draw_header_pane(ui, cx, origin, &mut anchors);
            ui.add_space(4.0);
            anchors.push(("variables".to_owned(), ui.cursor().min.y - origin));
            let top = ui.cursor().min.y;
            let count = self.header.vars.len().to_string();
            let vars_key = self.key("vars");
            let vars = Section::new(&vars_key, "Variables")
                .count(Tx::raw(if self.header.vars.is_empty() { "" } else { count.as_str() }))
                .action(Icon::Plus, "Add a variable")
                .tip(if cx.graph.is_behaviour() { "The graph's own variables" } else { "The quest's own variables" })
                .show(ui);
            if vars.open || vars.action.is_some() {
                self.draw_vars(ui, cx, vars.action.is_some());
            }
            ui.add_space(4.0);
            let gcount = self.globals.len().to_string();
            let globals_key = self.key("globals");
            let globals = Section::new(&globals_key, "Global")
                .count(Tx::raw(if self.globals.is_empty() { "" } else { gcount.as_str() }))
                .action(Icon::Plus, "Add a global variable")
                .tip(tr!("The module's variables, shared by every graph of it: declared in mod.ltx [vars] and saved there as you edit. global.set and global.add write one, the global condition reads it, {{global:name}} shows it in a text."))
                .show(ui);
            if globals.open || globals.action.is_some() {
                self.draw_globals(ui, cx, globals.action.is_some());
            }
            vars_span = (top, ui.cursor().min.y);
        });
        if req.as_deref() == Some("bottom") && mine {
            self.bottom_next = Some("graph");
            taken = true;
        }
        if taken {
            self.scroll = None;
        }
        self.anchors = anchors;
        self.graph_scroll =
            [out.state.clamped_offset().y, (out.content_size.y - out.inner_rect.height()).max(0.0)];
        self.vars_scroll = self.graph_scroll;
        let (a, b) = (vars_span.0.max(rect.min.y), vars_span.1.min(rect.max.y));
        self.vars_pane = (b > a).then(|| Rect::from_min_max(pos2(rect.min.x, a), pos2(rect.max.x, b)));
        self.vars_share = ((b - a) / rect.height().max(1.0)).max(0.0);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    fn draw_header_pane(
        &mut self,
        ui: &mut Ui,
        cx: &mut PanelCx<'_>,
        origin: f32,
        anchors: &mut Vec<(String, f32)>,
    ) {
        let behaviour = cx.graph.is_behaviour();
        let mut ch = false;
        let (open, _) = fold(
            ui,
            &self.key("head"),
            if behaviour { "Behaviour Graph" } else { "Quest" },
            None,
            true,
            None,
        );
        if open {
            let help = if behaviour {
                "For the author only - a behaviour graph shows the player nothing"
            } else {
                "What the player reads in the PDA"
            };
            let mut title = self.header.title.clone();
            let mut acti = usize::from(self.header.activation == "manual");
            let mut id = self.header.id.clone();
            {
                let mut pcx = self.param_cx(ui, cx, ROLE_HEADER, true);
                if params::draw_text(ui, &mut pcx, "title", &Row::new("Title", help, 0), &mut title, false) {
                    ch = true;
                }
                if !behaviour {
                    let changed = params::grid(ui, ROLE_HEADER, "activation", |g| {
                        g.row("Activation", "Auto: the quest starts with the game. Manual: another quest or a script starts it", PropFlags::NONE, 0, |ui| gz_ui::segmented(ui, &mut acti, &["Auto", "Manual"], None)).unwrap_or(false)
                    });
                    ch |= changed;
                }
                let (more, _) = fold(ui, &self.key("head_more"), "More", None, false, None);
                if more {
                    let h = if behaviour {
                        "The graph's id: what behaviour.run names. a-z, 0-9 and _"
                    } else {
                        "The quest's id: unique in the module. a-z, 0-9 and _"
                    };
                    ch |= params::grid(ui, ROLE_HEADER, "id", |g| {
                        g.row("ID", h, PropFlags::NONE, 1, |ui| grid::text_field(ui, &mut id).changed())
                            .unwrap_or(false)
                    });
                }
            }
            if ch {
                self.header.title = title;
                if !behaviour {
                    self.header.activation = if acti == 1 { "manual".to_owned() } else { "auto".to_owned() };
                }
                self.header.id = id;
            }
        }
        anchors.push(("tasks".to_owned(), ui.cursor().min.y - origin));
        if behaviour {
            ch |= self.draw_roles(ui, cx);
        } else {
            ch |= self.draw_tasks(ui, cx, origin, anchors);
        }
        if ch {
            self.header_dirty = true;
        }
    }

    #[allow(clippy::too_many_lines)]
    fn draw_tasks(
        &mut self,
        ui: &mut Ui,
        cx: &mut PanelCx<'_>,
        origin: f32,
        anchors: &mut Vec<(String, f32)>,
    ) -> bool {
        let mut ch = false;
        let (open, add) = fold(ui, &self.key("tasks"), "Tasks", None, true, Some((Icon::Plus, "Add a task")));
        if add {
            let mut n = 1;
            while self.header.tasks.iter().any(|t| t.id == format!("task{n}")) {
                n += 1;
            }
            self.header.tasks.push(Task {
                id: format!("task{n}"),
                ty: "additional".to_owned(),
                title: Value::from(""),
                descr: Value::from(""),
                ..Task::default()
            });
            ch = true;
        }
        if !open {
            return ch;
        }
        let mut removed = None;
        let count = self.header.tasks.len();
        for i in 0..count {
            let Some(mut t) = self.header.tasks.get(i).cloned() else { continue };
            anchors.push((format!("task:{}", t.id), ui.cursor().min.y - origin));
            let head = if t.id.is_empty() { Tx::from("(no id)") } else { Tx::raw(t.id.clone()) };
            let task_key = self.key(&format!("task:{}", t.id));
            let (topen, x) = ui
                .push_id(i, |ui| {
                    fold(
                        ui,
                        &task_key,
                        head,
                        None,
                        true,
                        Some((Icon::Close, "Remove the task. Ctrl+Z brings it back")),
                    )
                })
                .inner;
            if x {
                removed = Some(i);
            }
            if !topen {
                continue;
            }
            let mut t_ch = false;
            ui.push_id(("task", i), |ui| {
                let mut refs = false;
                let mut rename = false;
                params::grid(ui, ROLE_TASKS, "id", |g| {
                    g.row("ID", "The task's id in the PDA and in saves. Rename changes every reference with it", PropFlags::READ_ONLY, 1, |ui| grid::read_only(ui, Tx::raw(t.id.as_str()), Clip::End));
                    refs = g.trailing(Icon::FindInProject, "Find the task's references in the project");
                    rename = g.trailing(Icon::Rename, "Rename the task and every reference to it");
                });
                if refs {
                    self.refs_task = Some(t.id.clone());
                }
                if rename {
                    self.rename_task = Some(t.id.clone());
                }
                let mut pcx = self.param_cx(ui, cx, ROLE_TASKS, true);
                t_ch |= params::draw_text(ui, &mut pcx, "title", &Row::new("Title", "The task's name in the PDA", 1), &mut t.title, false);
                t_ch |= params::draw_text(ui, &mut pcx, "descr", &Row::new("Description", "What the PDA says about the task", 1), &mut t.descr, true);
                let mut ty = t.ty.clone();
                let items = ["additional".to_owned(), "storyline".to_owned()];
                if params::grid(ui, ROLE_TASKS, "type", |g| {
                    g.row("Type", "Storyline tasks are listed first in the PDA", PropFlags::NONE, 1, |ui| {
                        let mut at = usize::from(ty == "storyline");
                        let c = grid::combo(ui, "type", &mut at, &[Tx::ctx("task_type", "additional"), Tx::ctx("task_type", "storyline")]);
                        if c {
                            ty = items.get(at).cloned().unwrap_or_default();
                        }
                        c
                    })
                    .unwrap_or(false)
                }) {
                    t.ty = ty;
                    t_ch = true;
                }
                t_ch |= params::npc_ref(ui, &mut pcx, "target", &Row::new("Target", "Where the task's marker points", 1), &mut t.target, true, false);
                let mut icon = t.icon.clone();
                if params::grid(ui, ROLE_TASKS, "icon", |g| g.row("Icon", "The PDA icon of the task: a texture id", PropFlags::NONE, 1, |ui| grid::text_field(ui, &mut icon).changed()).unwrap_or(false)) {
                    t.icon = icon;
                    t_ch = true;
                }
                drop(pcx);
                // meaningless without steps to wait for, so it lives with them
                if !t.objectives.is_empty() {
                    let mut ac = t.auto_complete;
                    if params::grid(ui, ROLE_TASKS, "auto", |g| {
                        g.row("Auto-complete", "The last completed objective completes the task. Off when the task ends on something its objectives do not cover", PropFlags::INLINE, 1, |ui| grid::switch(ui, &mut ac).changed()).unwrap_or(false)
                    }) {
                        t.auto_complete = ac;
                        t_ch = true;
                    }
                }
            });
            anchors.push((format!("task:{}/steps", t.id), ui.cursor().min.y - origin));
            let (mut up, mut down, mut drop_o) = (None, None, None);
            let n = t.objectives.len();
            for j in 0..n {
                let Some(mut o) = t.objectives.get(j).cloned() else { continue };
                let mut o_ch = false;
                ui.push_id(("obj", i, j), |ui| {
                    let mut oid = o.id.clone();
                    params::grid(ui, ROLE_TASKS, "oid", |g| {
                        if g.row(tr!("Objective {n}", n = j + 1), "The objective's id: what objective.* and task.* nodes name", PropFlags::NONE, 2, |ui| grid::text_field(ui, &mut oid).changed()).unwrap_or(false) {
                            o_ch = true;
                        }
                        if g.trailing(Icon::Delete, "Remove the objective") {
                            drop_o = Some(j);
                        }
                        if j + 1 < n && g.trailing(Icon::MoveDown, "Move down") {
                            down = Some(j);
                        }
                        if j > 0 && g.trailing(Icon::MoveUp, "Move up") {
                            up = Some(j);
                        }
                    });
                    o.id = oid;
                    let mut pcx = self.param_cx(ui, cx, ROLE_TASKS, true);
                    o_ch |= params::draw_text(ui, &mut pcx, "title", &Row::new("Title", "The objective's line in the PDA", 3), &mut o.title, false);
                    o_ch |= params::draw_text(ui, &mut pcx, "descr", &Row::new("Description", "", 3), &mut o.descr, true);
                    o_ch |= params::npc_ref(ui, &mut pcx, "target", &Row::new("Target", "Where the objective's marker points; unset: no marker", 3), &mut o.target, true, false);
                    drop(pcx);
                    let mut vis = o.visible;
                    if params::grid(ui, ROLE_TASKS, "vis", |g| {
                        g.row("Visible", "Off: the step runs but takes no room in the PDA. task.set_objective_visible turns it on later", PropFlags::INLINE, 3, |ui| grid::switch(ui, &mut vis).changed()).unwrap_or(false)
                    }) {
                        o.visible = vis;
                        o_ch = true;
                    }
                });
                if o_ch && let Some(slot) = t.objectives.get_mut(j) {
                    *slot = o;
                    t_ch = true;
                }
            }
            if let Some(j) = drop_o {
                t.objectives.remove(j);
                t_ch = true;
            } else if let Some(j) = up.filter(|&j| j > 0) {
                t.objectives.swap(j, j - 1);
                t_ch = true;
            } else if let Some(j) = down.filter(|&j| j + 1 < t.objectives.len()) {
                t.objectives.swap(j, j + 1);
                t_ch = true;
            }
            if ui.push_id(("add_obj", i), |ui| params::add_button(ui, "Add Objective", 2)).inner {
                let mut k = 1;
                while t.objective(&format!("step{k}")).is_some() {
                    k += 1;
                }
                t.objectives.push(Objective { id: format!("step{k}"), ..Objective::default() });
                t_ch = true;
            }
            if t_ch && let Some(slot) = self.header.tasks.get_mut(i) {
                *slot = t;
                ch = true;
            }
        }
        if let Some(i) = removed {
            self.header.tasks.remove(i);
            ch = true;
        }
        ch
    }

    /// The cast of a behaviour graph: named slots its caller fills.
    fn draw_roles(&mut self, ui: &mut Ui, cx: &mut PanelCx<'_>) -> bool {
        let mut ch = false;
        let (open, add) = fold(ui, &self.key("roles"), "Roles", None, true, Some((Icon::Plus, "Add a role")));
        if add {
            let mut n = 1;
            while self.header.roles.iter().any(|r| r.name == format!("role{n}")) {
                n += 1;
            }
            self.header.roles.push(Role {
                name: format!("role{n}"),
                title: Value::from(""),
                ..Role::default()
            });
            ch = true;
        }
        if !open {
            return ch;
        }
        let mut removed = None;
        for i in 0..self.header.roles.len() {
            let Some(mut r) = self.header.roles.get(i).cloned() else { continue };
            let head = match (r.name.is_empty(), r.many) {
                (true, false) => Tx::from("(no name)"),
                (true, true) => tr!("{role} (group)", role = Tx::from("(no name)")),
                (false, false) => Tx::raw(r.name.clone()),
                (false, true) => tr!("{role} (group)", role = r.name.as_str()),
            };
            let role_key = self.key(&format!("role{i}"));
            let (ropen, x) = ui
                .push_id(i, |ui| {
                    fold(
                        ui,
                        &role_key,
                        head,
                        None,
                        true,
                        Some((Icon::Close, "Remove the role. Ctrl+Z brings it back")),
                    )
                })
                .inner;
            if x {
                removed = Some(i);
            }
            if !ropen {
                continue;
            }
            let mut r_ch = false;
            ui.push_id(("role", i), |ui| {
                let mut name = r.name.clone();
                if params::grid(ui, ROLE_ROLES, "name", |g| g.row("Name", "What the steps call it: who = \"<name>\". a-z, 0-9 and _ only", PropFlags::NONE, 1, |ui| grid::text_field(ui, &mut name).changed()).unwrap_or(false)) {
                    r.name = name;
                    r_ch = true;
                }
                let more_key = self.key(&format!("role{i}/more"));
                let (more, _) = fold(ui, &more_key, "More", None, false, None);
                if more {
                    let mut pcx = self.param_cx(ui, cx, ROLE_ROLES, true);
                    r_ch |= params::draw_text(ui, &mut pcx, "title", &Row::new("Title", "For the author: shown where the caller binds the role. Never seen by the player", 2), &mut r.title, false);
                    let (mut many, mut optional) = (r.many, r.optional);
                    let mut b_ch = false;
                    params::grid(ui, ROLE_ROLES, "binding", |g| {
                        g.row("Binding", "Group: the role may hold several NPCs - a squad, a list. Optional: bound to nobody is not an error", PropFlags::NONE, 2, |ui| {
                            b_ch |= gz_ui::Switch::new(&mut many, Tx::ctx("role", "Group")).tip("The role may hold several NPCs - a squad, a list. Off: exactly one").show(ui).changed();
                            ui.add_space(16.0);
                            b_ch |= gz_ui::Switch::new(&mut optional, "Optional").tip("Bound to nobody is not an error: steps addressed to it do nothing").show(ui).changed();
                        });
                    });
                    if b_ch {
                        r.many = many;
                        r.optional = optional;
                        r_ch = true;
                    }
                    r_ch |= params::npc_ref(ui, &mut pcx, "default", &Row::new("Default", "Who fills the role when the caller names nobody. Unset: the caller has to say", 2), &mut r.default, false, false);
                }
            });
            if r_ch && let Some(slot) = self.header.roles.get_mut(i) {
                *slot = r;
                ch = true;
            }
        }
        if let Some(i) = removed {
            self.header.roles.remove(i);
            ch = true;
        }
        ch
    }

    /// The graph's variables: a list, the one picked in a form under it titled by its name; `add`:
    /// the section's + was pressed.
    fn draw_vars(&mut self, ui: &mut Ui, cx: &mut PanelCx<'_>, add_now: bool) {
        let mut ch = false;
        let mut add = add_now;
        if self.header.vars.is_empty() {
            add |= gz_ui::state::empty_state(
                ui,
                Icon::Variable,
                "No variables",
                "A variable's default value is also its type.",
                Some("Add Variable".into()),
            );
        } else {
            self.var_sel = self.var_sel.min(self.header.vars.len() - 1);
            let rows = self.header.vars.len().min(6) as f32;
            let mut drop = None;
            let mut pick = None;
            gz_ui::list::list(ui, "gz-ui-graph.vars", vec2(0.0, rows * 22.0 + 10.0), |l| {
                for (i, v) in self.header.vars.iter().enumerate() {
                    let label = format!(
                        "{}  \u{b7}  {} {}",
                        v.name,
                        params::type_name(&v.value),
                        params::value_text(&v.value)
                    );
                    if l.row(&format!("var{i}"), i == self.var_sel, |ui| {
                        gz_ui::text::label(
                            ui,
                            Tx::raw(label.as_str()),
                            Font::Body,
                            palette(ui.ctx()).text,
                            Clip::End,
                        );
                    })
                    .clicked()
                    {
                        pick = Some(i);
                    }
                    if l.row_actions(&[Icon::Delete], &["Remove the variable"]) == Some(0) {
                        drop = Some(i);
                    }
                }
            });
            if let Some(i) = pick
                && i != self.var_sel
            {
                // a name typed and left for another variable is a rename all the same
                let want = self.var_name_buf.trim().to_owned();
                let of = std::mem::take(&mut self.var_name_of);
                if !want.is_empty() && want != of && self.header.vars.iter().any(|v| v.name == of) {
                    if self.rename_var(cx, &of, &want) {
                        ch = true;
                    } else {
                        self.notes.push(format!("The variable '{want}' already exists"));
                    }
                }
                self.var_name_buf.clear();
                self.var_sel = i;
            }
            if let Some(i) = drop {
                self.header.vars.remove(i);
                self.var_sel = self.var_sel.min(self.header.vars.len().saturating_sub(1));
                ch = true;
            }
            if let Some(var) = self.header.vars.get(self.var_sel).cloned() {
                let i = self.var_sel;
                ui.add_space(4.0);
                gz_ui::text::label(
                    ui,
                    Tx::raw(var.name.as_str()),
                    Font::Strong,
                    palette(ui.ctx()).text,
                    Clip::End,
                );
                if self.var_name_buf.is_empty() || !ui.ctx().egui_wants_keyboard_input() {
                    self.var_name_buf.clone_from(&var.name);
                    self.var_name_of.clone_from(&var.name);
                }
                let mut buf = self.var_name_buf.clone();
                let mut commit = false;
                ui.push_id(("var", i), |ui| {
                    params::grid(ui, ROLE_VARS, "name", |g| {
                        g.row(
                            "Name",
                            "Renaming changes every var node and condition that names it",
                            PropFlags::NONE,
                            0,
                            |ui| {
                                let r = grid::text_field(ui, &mut buf);
                                commit = r.lost_focus();
                            },
                        );
                    });
                });
                self.var_name_buf = buf;
                if commit {
                    let want = self.var_name_buf.trim().to_owned();
                    if !want.is_empty() && want != var.name {
                        if self.rename_var(cx, &var.name, &want) {
                            ch = true;
                        } else {
                            self.notes.push(format!("The variable '{want}' already exists"));
                        }
                    }
                }
                let mut value = var.value.clone();
                let mut cur = if value.is_bool() {
                    0
                } else if value.is_number() {
                    1
                } else {
                    2
                };
                let was = cur;
                let mut type_ch = false;
                ui.push_id(("var_type", i), |ui| {
                    params::grid(ui, ROLE_VARS, "type", |g| {
                        g.row(
                            "Type",
                            "Bool, number or text: the type of the default, and of every value it takes",
                            PropFlags::NONE,
                            0,
                            |ui| {
                                type_ch = gz_ui::segmented(ui, &mut cur, &["Bool", "Number", "Text"], None);
                            },
                        );
                    });
                });
                if type_ch && cur != was {
                    value = params::convert(&value, cur);
                    ch = true;
                }
                let as_type = value.clone();
                if ui
                    .push_id(("var_default", i), |ui| {
                        params::typed_value(
                            ui,
                            ROLE_VARS,
                            "default",
                            &Row::new("Default", "The value the variable has when the graph starts", 0),
                            &mut value,
                            &as_type,
                        )
                    })
                    .inner
                {
                    ch = true;
                }
                if let Some(slot) = self.header.vars.get_mut(i)
                    && slot.value != value
                {
                    slot.value = value;
                }
            }
        }
        if add {
            let mut n = self.header.vars.len() + 1;
            while self.header.vars.iter().any(|v| v.name == format!("var{n}")) {
                n += 1;
            }
            self.header.vars.push(Var { name: format!("var{n}"), value: Value::Bool(false) });
            self.var_sel = self.header.vars.len() - 1;
            ch = true;
        }
        if ch {
            self.header_dirty = true;
        }
    }

    /// Renames a declared variable and every var.set / var.add / var reference to it, as one step
    /// of the whole graph. False when the name is taken.
    fn rename_var(&mut self, cx: &mut PanelCx<'_>, from: &str, to: &str) -> bool {
        if to.is_empty() || from == to || self.header.vars.iter().any(|v| v.name == to) {
            return false;
        }
        for v in &mut self.header.vars {
            if v.name == from {
                to.clone_into(&mut v.name);
            }
        }
        let mut g = cx.graph.clone();
        for v in &mut g.vars {
            if v.name == from {
                to.clone_into(&mut v.name);
            }
        }
        for n in g.nodes.iter_mut() {
            for a in n.on_enter.iter_mut().chain(n.on_exit.iter_mut()) {
                if (a.kind == "var.set" || a.kind == "var.add")
                    && let Some(nm) = a.params.table_mut().and_then(|t| t.get_mut("name"))
                    && nm.as_str() == Some(from)
                {
                    *nm = Value::from(to);
                }
                rename_var_in(&mut a.params, from, to);
            }
            for c in &mut n.cond {
                if c.kind == "var"
                    && let Some(nm) = c.params.table_mut().and_then(|t| t.get_mut("name"))
                    && nm.as_str() == Some(from)
                {
                    *nm = Value::from(to);
                }
                rename_var_in(&mut c.params, from, to);
            }
        }
        // the header goes with it: what is staged is in the replaced graph already
        g.id.clone_from(&self.header.id);
        g.title = self.header.title.clone();
        g.activation.clone_from(&self.header.activation);
        g.tasks = self.header.tasks.iter().cloned().collect();
        g.roles.clone_from(&self.header.roles);
        g.vars.clone_from(&self.header.vars);
        cx.ops.push(Op::Doc(DocOp::Replace(g)));
        self.header_dirty = false;
        self.header_edit = None;
        self.node_edit = u64::MAX;
        true
    }

    /// The module's global variables under the graph's own: a list, the one picked in a form
    /// titled by its name; `add_now`: the section's + was pressed.
    #[allow(clippy::too_many_lines)]
    fn draw_globals(&mut self, ui: &mut Ui, cx: &mut PanelCx<'_>, add_now: bool) {
        if self.globals_rev != Some(cx.data.globals_rev) && !self.globals_dirty {
            self.globals_rev = Some(cx.data.globals_rev);
            self.globals = cx.data.globals.iter().map(|(n, v)| (n.clone(), global_value(v))).collect();
        }
        let mut ch = false;
        let mut add = add_now && cx.data.has_project;
        if !cx.data.has_project {
            gz_ui::text::status(ui, Tone::Muted, "No project: nowhere to keep them.");
        } else if self.globals.is_empty() {
            add |= gz_ui::state::empty_state(
                ui,
                Icon::Variable,
                "No global variables",
                "A value every graph of the module sees - the night, a flag, a count.",
                Some("Add Global".into()),
            );
        } else {
            self.global_sel = self.global_sel.min(self.globals.len() - 1);
            let rows = self.globals.len().min(6) as f32;
            let mut drop = None;
            let mut pick = None;
            gz_ui::list::list(ui, "gz-ui-graph.globals", vec2(0.0, rows * 22.0 + 10.0), |l| {
                for (i, (name, value)) in self.globals.iter().enumerate() {
                    let label =
                        format!("{name}  \u{b7}  {} {}", params::type_name(value), global_text(value));
                    if l.row(&format!("global{i}"), i == self.global_sel, |ui| {
                        gz_ui::text::label(
                            ui,
                            Tx::raw(label.as_str()),
                            Font::Body,
                            palette(ui.ctx()).text,
                            Clip::End,
                        );
                    })
                    .clicked()
                    {
                        pick = Some(i);
                    }
                    if l.row_actions(
                        &[Icon::Delete],
                        &["Remove the global variable - the graphs that name it show an error until they stop"],
                    ) == Some(0)
                    {
                        drop = Some(i);
                    }
                }
            });
            if let Some(i) = pick {
                self.global_sel = i;
            }
            if let Some(i) = drop {
                self.globals.remove(i);
                self.global_sel = self.global_sel.min(self.globals.len().saturating_sub(1));
                ch = true;
            }
            if let Some((name, value)) = self.globals.get(self.global_sel).cloned() {
                let i = self.global_sel;
                ui.add_space(4.0);
                gz_ui::text::label(
                    ui,
                    Tx::raw(name.as_str()),
                    Font::Strong,
                    palette(ui.ctx()).text,
                    Clip::End,
                );
                if self.global_name_buf.is_empty() || !ui.ctx().egui_wants_keyboard_input() {
                    self.global_name_buf.clone_from(&name);
                }
                let mut buf = self.global_name_buf.clone();
                let mut commit = false;
                ui.push_id(("global", i), |ui| {
                    params::grid(ui, ROLE_GLOBALS, "name", |g| {
                        g.row("Name", "Letters, digits and _. A rename changes the declaration only: every graph that names the old one shows an error until it names the new one", PropFlags::NONE, 0, |ui| {
                            commit = grid::text_field(ui, &mut buf).lost_focus();
                        });
                    });
                });
                self.global_name_buf = buf;
                if commit {
                    let want = self.global_name_buf.trim().to_owned();
                    if !want.is_empty() && want != name {
                        if let Some(why) = gz_mod::global_name_refusal(&want) {
                            // the manifest's own sentence, as a sentence of the panel
                            let mut note = why;
                            if let Some(first) = note.get(..1) {
                                let upper = first.to_uppercase();
                                note.replace_range(..1, &upper);
                            }
                            self.notes.push(note);
                        } else if self.globals.iter().any(|(n, _)| *n == want) {
                            self.notes.push(format!("The global variable '{want}' already exists"));
                        } else if let Some(slot) = self.globals.get_mut(i) {
                            slot.0 = want;
                            ch = true;
                        }
                    }
                }
                let mut v = value.clone();
                let mut cur = if v.is_bool() {
                    0
                } else if v.is_number() {
                    1
                } else {
                    2
                };
                let was = cur;
                let mut type_ch = false;
                ui.push_id(("global_type", i), |ui| {
                    params::grid(ui, ROLE_GLOBALS, "type", |g| {
                        g.row(
                            "Type",
                            "Bool, number or text: the type of the default, and of every value it takes",
                            PropFlags::NONE,
                            0,
                            |ui| {
                                type_ch = gz_ui::segmented(ui, &mut cur, &["Bool", "Number", "Text"], None);
                            },
                        );
                    });
                });
                if type_ch && cur != was {
                    v = params::convert(&v, cur);
                    ch = true;
                }
                let as_type = v.clone();
                if ui
                    .push_id(("global_default", i), |ui| {
                        params::typed_value(
                            ui,
                            ROLE_GLOBALS,
                            "default",
                            &Row::new(
                                "Default",
                                "The value a new game starts with; a save keeps what the graphs made of it",
                                0,
                            ),
                            &mut v,
                            &as_type,
                        )
                    })
                    .inner
                {
                    ch = true;
                }
                if let Some(slot) = self.globals.get_mut(i) {
                    slot.1 = v;
                }
            }
        }
        if add {
            let mut n = self.globals.len() + 1;
            while self.globals.iter().any(|(g, _)| *g == format!("global{n}")) {
                n += 1;
            }
            self.globals.push((format!("global{n}"), Value::Bool(false)));
            self.global_sel = self.globals.len() - 1;
            ch = true;
        }
        if ch {
            self.globals_dirty = true;
        }
        let _ = cx;
    }

    /// Writes the globals into mod.ltx `[vars]` through the manifest's store.
    fn save_globals(&mut self, editor: &Arc<Editor>) {
        self.globals_dirty = false;
        let lines: Vec<String> = self
            .globals
            .iter()
            .map(|(n, v)| gz_mod::doc::entry_line(n, &global_text(v), gz_mod::doc::key_width("vars")))
            .collect();
        let ed = editor.clone();
        let origin = editor.ui_origin();
        let _task = editor.exec.spawn(async move {
            let result = match gz_mod::ManifestService::of(&ed).await {
                Ok(m) => m.edit(vec![gz_mod::Edit::lines("vars", lines)], origin).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                ed.log.error(format!(
                    "[nq] mod.ltx could not be written - the global variables were not saved: {}",
                    e.message
                ));
            }
        });
        // read back from the manifest once it moved
        self.globals_rev = None;
    }

    // ---- the Details panel --------------------------------------------------------------------

    /// The Details panel into `rect`.
    #[allow(clippy::too_many_lines)]
    pub fn draw_details(&mut self, ui: &mut Ui, rect: Rect, cx: &mut PanelCx<'_>) {
        self.sync_node(cx, ui.ctx().cumulative_pass_nr());
        self.sync_header(cx);
        let ctx = ui.ctx().clone();
        let spacing = 4.0;
        let footer = 8.0 * 2.0 + Font::Small.line() * 2.0 + 2.0;
        let column = (rect.height() - footer - spacing).max(60.0);
        self.details_column = column;
        self.node_pane = None;
        self.action_pane = None;
        let mut child = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.details").max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = spacing;

        // a newly selected action is brought into view in the node pane
        let reveal = if self.node_id.is_empty() {
            String::new()
        } else {
            format!("{}|{}", self.node_id, cx.view.slot)
        };
        if reveal != self.reveal_key {
            self.reveal_frames = if self.node_id.is_empty() || cx.view.slot.is_empty() { 0 } else { 3 };
            self.reveal_open = self.reveal_frames > 0;
            self.reveal_key = reveal;
            self.reveal_at = ui.input(|i| i.time);
        }

        if self.node_id.is_empty() || cx.graph.find_node(&self.node_id).is_none() {
            self.node_scroll = [0.0, 0.0];
            self.action_scroll = [0.0, 0.0];
            let n = cx.view.selection.len();
            let (title, text_): (Tx<'static>, Tx<'static>) = if n > 1 {
                (tr!("{n} nodes selected", n = n), "Select one node to edit it.".into())
            } else if cx.graph.is_behaviour() {
                // a behaviour graph never says "quest"
                (
                    "Nothing selected".into(),
                    "Select a node to edit it. The graph's own settings are in the Graph panel.".into(),
                )
            } else {
                (
                    "Nothing selected".into(),
                    "Select a node to edit it. The quest's own settings are in the Graph panel.".into(),
                )
            };
            gz_ui::state::empty_state(&mut child, Icon::Info, title, text_, None);
            ui.allocate_rect(rect, egui::Sense::hover());
            return;
        }
        let has_action = Self::has_action(cx.view);
        let mut col = child.new_child(
            UiBuilder::new()
                .id_salt("column")
                .max_rect(Rect::from_min_size(rect.min, vec2(rect.width(), column))),
        );
        col.spacing_mut().item_spacing.y = spacing;
        let node_h = if has_action {
            if let Some(f) = self.pending_action.take() {
                gz_ui::layout::set(&ctx, ACTION_SPLIT, f * column);
            }
            gz_ui::splitter::splitter(&mut col, ACTION_SPLIT, false, 100.0, 100.0, column * 0.5).size
        } else {
            column
        };
        let node_r = Rect::from_min_size(rect.min, vec2(rect.width(), node_h));
        let action_r = Rect::from_min_max(
            pos2(rect.min.x, rect.min.y + node_h + spacing),
            pos2(rect.max.x, rect.min.y + column),
        );
        self.node_pane = Some(node_r);
        if has_action {
            self.action_pane = Some(action_r);
        }
        self.action_share = if has_action { node_h / column.max(1.0) } else { 1.0 };

        // the help footer shows what the grid drawn last had under the cursor: that pane last
        let pointer_on_node = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| node_r.contains(p));
        if has_action && pointer_on_node {
            self.action_pane_draw(&mut child, action_r, cx);
            self.node_pane_draw(&mut child, node_r, cx);
        } else {
            self.node_pane_draw(&mut child, node_r, cx);
            if has_action {
                self.action_pane_draw(&mut child, action_r, cx);
            } else {
                self.action_scroll = [0.0, 0.0];
            }
        }
        let foot = Rect::from_min_max(pos2(rect.min.x, rect.min.y + column + spacing), rect.max);
        let mut fui = child.new_child(UiBuilder::new().id_salt("footer").max_rect(foot));
        grid::help_footer(&mut fui);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    fn node_pane_draw(&mut self, ui: &mut Ui, r: Rect, cx: &mut PanelCx<'_>) {
        // a pane that changed its height (the split moved) shows the selected action again
        if (r.height() - self.reveal_pane_h).abs() > 0.5 {
            if !cx.view.slot.is_empty() && ui.input(|i| i.time) - self.reveal_at < 1.0 {
                self.reveal_frames = self.reveal_frames.max(3);
            }
            self.reveal_pane_h = r.height();
        }
        let mut pane = ui.new_child(UiBuilder::new().id_salt("node_pane").max_rect(r));
        pane.set_clip_rect(r.intersect(ui.clip_rect()));
        let mut sa = ScrollArea::vertical().id_salt("gz-ui-graph.node").auto_shrink([false, false]);
        if self.bottom_next == Some("node") {
            sa = sa.vertical_scroll_offset(f32::MAX / 4.0);
            self.bottom_next = None;
        }
        let req = self.scroll.clone();
        if req.as_deref() == Some("top") {
            sa = sa.vertical_scroll_offset(0.0);
            self.scroll = None;
        }
        // each node's fields under an id of its own: a field focused on one node is not the same
        // field of the next one, and nothing typed for one reaches the other
        let scope = self.node_id.clone();
        let out = sa
            .show(&mut pane, |ui| ui.push_id(("node", scope.as_str()), |ui| self.draw_node_section(ui, cx)));
        if req.as_deref() == Some("bottom") {
            self.bottom_next = Some("node");
            self.scroll = None;
        }
        self.node_scroll =
            [out.state.clamped_offset().y, (out.content_size.y - out.inner_rect.height()).max(0.0)];
    }

    fn action_pane_draw(&mut self, ui: &mut Ui, r: Rect, cx: &mut PanelCx<'_>) {
        let mut pane = ui.new_child(UiBuilder::new().id_salt("action_pane").max_rect(r));
        pane.set_clip_rect(r.intersect(ui.clip_rect()));
        let mut sa = ScrollArea::vertical().id_salt("gz-ui-graph.action").auto_shrink([false, false]);
        if self.bottom_next == Some("action") {
            sa = sa.vertical_scroll_offset(f32::MAX / 4.0);
            self.bottom_next = None;
        }
        let req = self.scroll.clone();
        if req.as_deref() == Some("action:top") {
            sa = sa.vertical_scroll_offset(0.0);
            self.scroll = None;
        }
        let scope = (self.node_id.clone(), cx.view.slot.clone());
        let out = sa.show(&mut pane, |ui| {
            ui.push_id(("action", scope.0.as_str(), scope.1.as_str()), |ui| self.draw_action_section(ui, cx))
        });
        if req.as_deref() == Some("action:bottom") {
            self.bottom_next = Some("action");
            self.scroll = None;
        }
        self.action_scroll =
            [out.state.clamped_offset().y, (out.content_size.y - out.inner_rect.height()).max(0.0)];
    }

    #[allow(clippy::too_many_lines)]
    fn draw_node_section(&mut self, ui: &mut Ui, cx: &mut PanelCx<'_>) {
        let behaviour = cx.graph.is_behaviour();
        let p = palette(ui.ctx());
        let cat = cx.cat;
        let trig = cat.is_trigger(&self.node.kind);
        // the catalog's English
        let title = Tx::ui_in("kind", look::kind_title(cat, &self.node.kind, behaviour));
        local::local_pane_header(
            ui,
            look::kind_icon(cat, &self.node.kind),
            look::kind_color(&p, cat, &self.node.kind, 1.0),
            title,
            &self.node.id,
            0.0,
        );
        // what is wrong with the node, first: each line names a parameter and says what to do
        for pr in cx.data.problems.iter().filter(|pr| pr.node == self.node_id.as_str()) {
            // the validator's English as a sentence; its code and place are the tooltip's
            let tone = if pr.is_error() { Tone::Error } else { Tone::Warning };
            gz_ui::text::status(ui, tone, Tx::ui(pr.sentence()))
                .tip(Tx::raw(look::problem_line(ui.ctx(), pr)));
        }
        let mut ch = false;
        let (open, _) = fold(ui, &self.key("node"), "Node", None, true, None);
        if open {
            // the id: renamed through the document so references follow
            if self.id_buf_of != self.node_id || !ui.ctx().egui_wants_keyboard_input() {
                self.id_buf = self.node_id.to_string();
                self.id_buf_of = self.node_id.clone();
            }
            let mut buf = self.id_buf.clone();
            let mut commit = false;
            let focus = std::mem::take(&mut self.focus_rename);
            params::grid(ui, ROLE_NODE, "id", |g| {
                g.row(
                    "ID",
                    "The node's id: what links, bookmarks and other nodes name. F2 renames it",
                    PropFlags::NONE,
                    0,
                    |ui| {
                        let r = grid::text_field(ui, &mut buf);
                        if focus {
                            r.request_focus();
                        }
                        if r.has_focus() {
                            self.id_focus_pass = ui.ctx().cumulative_pass_nr();
                        }
                        commit = r.lost_focus();
                    },
                );
            });
            self.id_buf = buf;
            if commit {
                let nid = self.id_buf.trim().to_owned();
                if nid != self.node_id.as_str() && !nid.is_empty() {
                    self.commit_node(cx);
                    cx.ops.push(Op::Doc(DocOp::RenameNode {
                        id: self.node_id.to_string(),
                        new_id: nid.clone(),
                    }));
                    cx.view.select_only(&nid);
                    self.node_id = SmolStr::new(&nid);
                    self.node.id = SmolStr::new(&nid);
                    self.node_edit = u64::MAX;
                }
            }
            // the kind
            let desc = look::kind_desc(cat, &self.node.kind, behaviour);
            let mut kind = self.node.kind.to_string();
            let changed = params::grid(ui, ROLE_NODE, "kind", |g| {
                g.row(
                    "Kind",
                    if desc.is_empty() {
                        Tx::from("What the node does. Changing it resets the parameters")
                    } else {
                        Tx::ui_in("kind", desc.clone())
                    },
                    PropFlags::NONE,
                    0,
                    |ui| {
                        look::kind_combo(
                            ui,
                            "node_kind",
                            cat,
                            if trig { USE_TRIGGER } else { USE_MAIN },
                            behaviour,
                            &mut kind,
                        )
                    },
                )
                .unwrap_or(false)
            });
            if changed && kind != self.node.kind.as_str() {
                self.node.kind = SmolStr::new(&kind);
                self.node.params = Value::table();
                self.node.once_set = false;
                // the parameters reset, not the wiring: a pin the new kind still declares keeps its edge
                let k = cat.find(&kind);
                self.node.out.retain(|p| k.is_some_and(|k| k.has_pin(&p.name)));
                ch = true;
            }
            let k = cat.find(&self.node.kind).cloned();
            if !trig && let Some(k) = &k {
                let mut once = if self.node.once_set { self.node.once } else { k.once_default };
                let flags = PropFlags::INLINE
                    | if self.node.once_set { PropFlags::MODIFIED } else { PropFlags::NONE };
                let mut reset = false;
                let toggled = params::grid(ui, ROLE_NODE, "once", |g| {
                    let t = g.row("Once", "Runs once: after it has run, the node is passed over. The kind decides when unset", flags, 0, |ui| grid::switch(ui, &mut once).changed()).unwrap_or(false);
                    reset = g.was_reset();
                    t
                });
                if toggled {
                    // back to what the kind does drops the override
                    self.node.once = once;
                    self.node.once_set = once != k.once_default;
                    ch = true;
                }
                if reset {
                    self.node.once_set = false;
                    self.node.once = k.once_default;
                    ch = true;
                }
            }
            match &k {
                Some(k) => {
                    let mut params_v = self.node.params.clone();
                    let mut pcx = self.param_cx(ui, cx, ROLE_NODE, false);
                    if params::draw_params(ui, &mut pcx, k, &mut params_v, 0) {
                        self.node.params = params_v;
                        ch = true;
                    }
                }
                None => {
                    params::grid(ui, ROLE_NODE, "unknown", |g| {
                        g.row("Kind", "", PropFlags::INVALID | PropFlags::READ_ONLY, 0, |ui| {
                            grid::read_only(ui, Tx::raw(kind.as_str()), Clip::End)
                        });
                        g.message(Tone::Error, "Unknown kind - not in the catalog");
                    });
                }
            }
            let mut comment = self.node.comment.clone();
            if params::grid(ui, ROLE_NODE, "comment", |g| {
                g.row("Comment", "For the author: the player never sees it", PropFlags::NONE, 0, |ui| {
                    grid::text_field(ui, &mut comment).changed()
                })
                .unwrap_or(false)
            }) {
                self.node.comment = comment;
                ch = true;
            }
        }

        // the conditions: all main and trigger kinds may carry them
        let count = (!self.node.cond.is_empty()).then(|| self.node.cond.len().to_string());
        let (copen, _) = fold(ui, &self.key("conds"), "Conditions", count.as_deref(), true, None);
        if copen {
            let mut conds = self.node.cond.clone();
            let mut pcx = self.param_cx(ui, cx, ROLE_NODE, false);
            if params::cond_list(ui, &mut pcx, "nc", 0, &mut conds, 0) {
                self.node.cond = conds;
                ch = true;
            }
        }

        // the actions, in order: a row opens one under the node
        if !trig {
            let total = self.node.on_enter.len() + self.node.on_exit.len();
            if self.reveal_open {
                section::set_open(ui.ctx(), &self.key("acts"), true);
                self.reveal_open = false;
            }
            let count = (total > 0).then(|| total.to_string());
            let (aopen, _) = fold(ui, &self.key("acts"), "Actions", count.as_deref(), true, None);
            if aopen {
                for (exit, list) in [(false, &self.node.on_enter), (true, &self.node.on_exit)] {
                    for (i, a) in list.iter().enumerate() {
                        let sel = format!("{}:{i}", if exit { "exit" } else { "enter" });
                        let where_ =
                            if exit { tr!("On exit {n}", n = i + 1) } else { tr!("On enter {n}", n = i + 1) };
                        let what = Tx::ui_in("kind", look::kind_title(cat, &a.kind, behaviour));
                        let selected = cx.view.slot == sel;
                        let spec = NavRowSpec {
                            icon: look::kind_icon(cat, &a.kind),
                            count: Some(where_),
                            tip: if exit {
                                "Runs when the node is left. Click to edit it below".into()
                            } else {
                                "Runs when the node is entered. Click to edit it below".into()
                            },
                            ..NavRowSpec::default()
                        };
                        let top = ui.cursor().min.y;
                        if nav_row(ui, &format!("act_{sel}"), what, selected, &spec).clicked {
                            cx.view.slot.clone_from(&sel);
                        }
                        if selected && self.reveal_frames > 0 {
                            let r = Rect::from_min_max(
                                pos2(ui.min_rect().min.x, top),
                                pos2(ui.min_rect().max.x, top + gz_ui::theme::metrics::ROW),
                            );
                            if ui.clip_rect().contains_rect(r) {
                                self.reveal_frames = 0;
                            } else {
                                // at once: a view asked over MCP is answered from the frame that shows it
                                ui.scroll_to_rect_animation(
                                    r,
                                    Some(Align::Center),
                                    egui::style::ScrollAnimation::none(),
                                );
                            }
                        }
                    }
                }
                let mut add_at = None;
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let r = gz_ui::Button::new("Add Action").kind(gz_ui::Kind::Ghost).show(ui);
                    if r.clicked() {
                        add_at = Some(r.rect.left_bottom());
                    }
                });
                if let Some(at) = add_at {
                    let pid = ui.make_persistent_id("gz-ui-graph.add_action_slot");
                    self.pick.anchor = at;
                    egui::Popup::open_id(ui.ctx(), pid);
                }
                let pid = ui.make_persistent_id("gz-ui-graph.add_action_slot");
                let anchor = Rect::from_min_size(self.pick.anchor, egui::Vec2::ZERO);
                let mut chosen = None;
                gz_ui::menu::menu_popup(ui.ctx(), pid, anchor, ui.layer_id(), |ui| {
                    if gz_ui::menu_item(ui, "On Enter...", gz_ui::MenuItem::default()) {
                        chosen = Some(false);
                    }
                    if gz_ui::menu_item(ui, "On Exit...", gz_ui::MenuItem::default()) {
                        chosen = Some(true);
                    }
                });
                if let Some(exit) = chosen {
                    self.add_action = Some((self.node_id.to_string(), exit, anchor.min));
                }
            }
            self.reveal_frames = self.reveal_frames.saturating_sub(1);
        }

        // where the pins lead: read here, wired on the canvas
        if self.node.out.iter().any(|p| !p.targets.is_empty()) || !self.node.out.is_empty() {
            let (oopen, _) = fold(ui, &self.key("outs"), "Outputs", None, false, None);
            if oopen {
                for (i, pin) in self.node.out.iter().enumerate() {
                    let tg = pin.targets.iter().map(SmolStr::as_str).collect::<Vec<_>>().join(", ");
                    params::text_row(
                        ui,
                        ROLE_NODE,
                        ("out", i),
                        &Row::new(Tx::ui_in("pin", pin.name.to_string()), "", 0),
                        if tg.is_empty() { Tx::from("(nothing)") } else { Tx::raw(tg.as_str()) },
                    );
                }
            }
        }
        if ch {
            self.node_dirty = true;
        }
        let _ = text::preview;
    }

    fn draw_action_section(&mut self, ui: &mut Ui, cx: &mut PanelCx<'_>) {
        let Some((exit, index)) = split_slot(&cx.view.slot) else { return };
        let list_len = if exit { self.node.on_exit.len() } else { self.node.on_enter.len() };
        if index >= list_len {
            cx.view.slot.clear();
            return;
        }
        let behaviour = cx.graph.is_behaviour();
        let cat = cx.cat;
        let p = palette(ui.ctx());
        let Some(mut a) = (if exit { &self.node.on_exit } else { &self.node.on_enter }).get(index).cloned()
        else {
            return;
        };
        let where_ =
            if exit { tr!("On exit {n}", n = index + 1) } else { tr!("On enter {n}", n = index + 1) };
        let where_ = where_.text(ui.ctx());
        let title = Tx::ui_in("kind", look::kind_title(cat, &a.kind, behaviour));
        let head = ui.cursor().min;
        let tools = 3.0 * 26.0;
        local::local_pane_header(
            ui,
            look::kind_icon(cat, &a.kind),
            look::kind_color(&p, cat, &a.kind, 1.0),
            title,
            &where_,
            tools,
        );
        let right = ui.max_rect().max.x;
        let (mut up, mut down, mut remove) = (false, false, false);
        let tools_r = Rect::from_min_size(pos2(right - tools, head.y + 2.0), vec2(tools, 24.0));
        let mut tui = ui.new_child(
            UiBuilder::new()
                .id_salt("action_tools")
                .max_rect(tools_r)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        tui.spacing_mut().item_spacing.x = 2.0;
        up = gz_ui::icon_button(&mut tui, Icon::MoveUp, "Move up", false, index > 0).clicked() || up;
        down = gz_ui::icon_button(&mut tui, Icon::MoveDown, "Move down", false, index + 1 < list_len)
            .clicked()
            || down;
        remove = gz_ui::icon_button(
            &mut tui,
            Icon::Delete,
            "Remove the action (Ctrl+Z brings it back)",
            false,
            true,
        )
        .clicked()
            || remove;
        let list = if exit { &mut self.node.on_exit } else { &mut self.node.on_enter };
        if up && index > 0 {
            list.swap(index, index - 1);
            cx.view.slot = format!("{}:{}", if exit { "exit" } else { "enter" }, index - 1);
            self.node_dirty = true;
            return;
        }
        if down && index + 1 < list.len() {
            list.swap(index, index + 1);
            cx.view.slot = format!("{}:{}", if exit { "exit" } else { "enter" }, index + 1);
            self.node_dirty = true;
            return;
        }
        if remove {
            list.remove(index);
            cx.view.slot.clear();
            self.node_dirty = true;
            return;
        }
        // when it runs: as the node is entered, or as it is left - switched here, the action goes
        // to the end of the other list
        let mut at = usize::from(exit);
        let switched = params::grid(ui, ROLE_ACTION, "runs", |g| {
            g.row(
                "Runs",
                "On enter: as the node is reached, before it waits. On exit: as the node is left, after what it waited for",
                PropFlags::NONE,
                0,
                |ui| gz_ui::segmented(ui, &mut at, &["On Enter", "On Exit"], None),
            )
            .unwrap_or(false)
        });
        if switched && (at == 1) != exit {
            let moved = list.remove(index);
            let other = if exit { &mut self.node.on_enter } else { &mut self.node.on_exit };
            other.push(moved);
            cx.view.slot = format!("{}:{}", if exit { "enter" } else { "exit" }, other.len() - 1);
            self.node_dirty = true;
            return;
        }
        let mut ch = false;
        let desc = look::kind_desc(cat, &a.kind, behaviour);
        let mut kind = a.kind.to_string();
        // the action asked for is brought into view; the keys stay the canvas's (a focused field
        // would take every shortcut, and Space would open the list)
        if std::mem::take(&mut self.focus_action) {
            self.reveal_frames = self.reveal_frames.max(3);
        }
        let changed = params::grid(ui, ROLE_ACTION, "kind", |g| {
            g.row(
                "Kind",
                if desc.is_empty() {
                    Tx::from("What the action does. Changing it resets the parameters")
                } else {
                    Tx::ui_in("kind", desc.clone())
                },
                PropFlags::NONE,
                0,
                |ui| look::kind_combo(ui, "action_kind", cat, USE_EXTRA, behaviour, &mut kind),
            )
            .unwrap_or(false)
        });
        if changed && kind != a.kind.as_str() {
            a.kind = SmolStr::new(&kind);
            a.params = Value::table();
            ch = true;
        }
        match cat.find(&a.kind).cloned() {
            Some(k) => {
                let mut pv = a.params.clone();
                let mut pcx = self.param_cx(ui, cx, ROLE_ACTION, false);
                if params::draw_params(ui, &mut pcx, &k, &mut pv, 0) {
                    a.params = pv;
                    ch = true;
                }
            }
            None => {
                params::grid(ui, ROLE_ACTION, "unknown", |g| {
                    g.row("Kind", "", PropFlags::INVALID | PropFlags::READ_ONLY, 0, |ui| {
                        grid::read_only(ui, Tx::raw(kind.as_str()), Clip::End)
                    });
                    g.message(Tone::Error, "Unknown kind - not in the catalog");
                });
            }
        }
        if ch {
            let list = if exit { &mut self.node.on_exit } else { &mut self.node.on_enter };
            if let Some(slot) = list.get_mut(index) {
                *slot = a;
            }
            self.node_dirty = true;
        }
    }
}
