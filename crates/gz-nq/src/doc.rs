//! An open document (`NqDoc`): the state of one `Store<GraphDoc>` and every edit operation of
//! `quest_apply`, `quest_edit` and the canvas as commands of that store.
//!
//! A command carries the catalog it is applied with, because what a node looks like (its pins
//! in order, its box, whether it is a trigger) is the catalog's. Undo, redo and dirtiness are
//! the store's; the view state (selection, the Details slot, the view's centre and zoom, the
//! bookmarks, the view history, the search) rides along and Undo keeps it.

use std::sync::Arc;
use std::time::SystemTime;

use gz_luadata::Value;
use gz_store::{Apply, CmdError, State};
use smol_str::SmolStr;

use crate::asset::{self, Action, Graph, Node, Task, Var};
use crate::catalog::Catalog;
use crate::index::{self, FindHit};
use crate::layout::{self, GAP_Y, GRID};
use crate::references;
use crate::text;

/// The canvas's zoom levels (`kZoom`); 6 is 1:1, 5 the floor of Frame All.
pub const ZOOM: [f32; 10] = [0.2, 0.3, 0.4, 0.5, 0.65, 0.75, 1.0, 1.25, 1.5, 2.0];
pub const ZOOM_DEFAULT: i32 = 6;
pub const ZOOM_FRAME_MIN: i32 = 5;
/// Views kept by the view history.
pub const HISTORY_LIMIT: usize = 64;
/// A graph of this many nodes gets a minimap.
pub const MINIMAP_MIN_NODES: usize = 12;

/// The factor of a zoom level.
#[must_use]
pub fn zoom_of(level: i32) -> f32 {
    let i = usize::try_from(level.clamp(0, ZOOM.len() as i32 - 1)).unwrap_or(6);
    ZOOM.get(i).copied().unwrap_or(1.0)
}

/// One remembered view: where the canvas looked and what was selected (`SViewState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub center: [f32; 2],
    pub zoom: i32,
    pub selection: Vec<SmolStr>,
    pub slot: String,
}

/// What a document's tab shows, kept with the document (`NqDoc`'s view fields, the canvas's
/// bookmarks and history, the window's search).
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub selection: Vec<SmolStr>,
    /// `enter:2` / `exit:0` / empty: the action the Details panel has open.
    pub slot: String,
    pub center: [f32; 2],
    pub zoom: i32,
    pub bookmarks: Vec<SmolStr>,
    pub back: Vec<ViewState>,
    pub forward: Vec<ViewState>,
    pub find_query: String,
    /// The node the search stands on.
    pub find_node: Option<SmolStr>,
    /// `none`, `problems` or `find`.
    pub drawer: SmolStr,
    pub minimap: bool,
    /// What the last frame drew: set by the tab.
    pub minimap_shown: bool,
    /// The canvas in px and the UI scale it was drawn at, set by the tab; `None` headless.
    pub canvas: Option<[f32; 2]>,
    pub dpi: f32,
    /// The canvas's last status line.
    pub status: String,
}

impl Default for View {
    fn default() -> Self {
        Self {
            selection: Vec::new(),
            slot: String::new(),
            center: [0.0, 0.0],
            zoom: ZOOM_DEFAULT,
            bookmarks: Vec::new(),
            back: Vec::new(),
            forward: Vec::new(),
            find_query: String::new(),
            find_node: None,
            drawer: SmolStr::new("none"),
            minimap: true,
            minimap_shown: false,
            canvas: None,
            dpi: 1.0,
            status: String::new(),
        }
    }
}

impl View {
    #[must_use]
    pub fn current(&self) -> ViewState {
        ViewState {
            center: self.center,
            zoom: self.zoom,
            selection: self.selection.clone(),
            slot: self.slot.clone(),
        }
    }

    /// Pushes the current view on the back history unless it is the last one (`RememberView`).
    fn remember(&mut self) {
        let state = self.current();
        if self.back.last() == Some(&state) {
            return;
        }
        self.back.push(state);
        if self.back.len() > HISTORY_LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    fn apply_state(&mut self, s: &ViewState, g: &Graph) {
        self.center = [layout::sane(s.center[0]), layout::sane(s.center[1])];
        self.zoom = s.zoom.clamp(0, ZOOM.len() as i32 - 1);
        self.selection = s.selection.iter().filter(|id| g.find_node(id).is_some()).cloned().collect();
        self.slot = if self.selection.len() == 1 { s.slot.clone() } else { String::new() };
    }

    /// Selected ids that no longer exist go, and so does the slot of a selection of other than
    /// one (`PruneSelection`); bookmarks of gone nodes go too.
    fn prune(&mut self, g: &Graph) {
        self.selection.retain(|id| g.find_node(id).is_some());
        if self.selection.len() != 1 {
            self.slot.clear();
        }
        self.bookmarks.retain(|id| g.find_node(id).is_some());
    }

    fn select_only(&mut self, id: &str) {
        self.selection.clear();
        if !id.is_empty() {
            self.selection.push(SmolStr::new(id));
        }
        self.slot.clear();
    }

    /// Centres a node at 1:1 or closer and selects it (`FrameNode`).
    fn frame_node(&mut self, cat: &Catalog, g: &Graph, id: &str) {
        let Some(n) = g.find_node(id) else { return };
        let s = layout::node_size(cat, n);
        let center = [n.pos[0] + s[0] * 0.5, n.pos[1] + s[1] * 0.5];
        let zoom = self.zoom.max(ZOOM_DEFAULT);
        let same = self.selection.len() == 1
            && self.selection.first().is_some_and(|s| s == id)
            && self.slot.is_empty();
        #[allow(clippy::float_cmp)]
        let moved = center != self.center || zoom != self.zoom;
        if moved || !same {
            self.remember();
        }
        self.center = center;
        self.zoom = zoom;
        self.select_only(id);
    }
}

/// The state of one open document.
#[derive(Clone, Debug)]
pub struct GraphDoc {
    pub graph: Graph,
    /// The reader's shape problems of the last load: what the model is missing.
    pub shape: Arc<[String]>,
    /// The file held what the reader could not represent: the model is only part of it.
    pub partial: bool,
    /// The file's write time at the last load or save.
    pub file_time: Option<SystemTime>,
    pub view: View,
}

impl Default for GraphDoc {
    fn default() -> Self {
        Self {
            graph: Graph::default(),
            shape: Arc::from(Vec::new()),
            partial: false,
            file_time: None,
            view: View::default(),
        }
    }
}

/// A canvas edit `quest_edit` reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasOp {
    /// Closes the chain over the selection: A->B->C becomes A->C.
    Bypass,
    Delete,
    Duplicate,
    ConnectNearest,
    SelectAll,
}

/// A view change: navigation, bookmarks, the history, the search, the minimap.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewOp {
    Select(Vec<String>),
    /// `none` or empty closes the action.
    Slot(String),
    Center([f32; 2]),
    Zoom(i32),
    FrameNode(String),
    HistoryBack,
    HistoryForward,
    HistoryClear,
    Bookmark {
        node: String,
        on: bool,
    },
    BookmarksClear,
    /// +1 or -1.
    BookmarkJump(i32),
    FindQuery(String),
    /// To result `index` (wrapping); -1 goes to the last.
    FindGo(i64),
    FindClear,
    Minimap(bool),
    /// What the tab drew: its canvas in px, the UI scale, whether the minimap was on screen.
    Drawn {
        canvas: Option<[f32; 2]>,
        dpi: f32,
        minimap_shown: bool,
    },
    Drawer(SmolStr),
}

/// What a command does to a document.
#[derive(Clone, Debug, PartialEq)]
pub enum DocOp {
    /// The file was read: the history starts over, clean.
    Load {
        graph: Graph,
        shape: Vec<String>,
        file_time: Option<SystemTime>,
    },
    /// `quest_write` on an open document: the whole graph replaced, one undo step.
    Replace(Graph),
    /// `quest_apply`: a list of operations, all or nothing.
    ApplyOps(Value),
    /// One operation.
    Apply(Value),
    AddNode(Node),
    RemoveNodes(Vec<String>),
    RenameNode {
        id: String,
        new_id: String,
    },
    RenameTask {
        from: String,
        to: String,
        path: String,
    },
    Connect {
        from: String,
        pin: String,
        to: String,
    },
    /// An empty `to` drops the whole pin, an empty `pin` every pin.
    Disconnect {
        from: String,
        pin: String,
        to: String,
    },
    SetPos {
        id: String,
        x: f32,
        y: f32,
        undo: bool,
    },
    Layout {
        only_missing: bool,
    },
    Canvas(CanvasOp),
    View(ViewOp),
    /// What is in memory is on disk now.
    Saved {
        file_time: Option<SystemTime>,
    },
}

/// A command of the document store: the operation and the catalog it is read with.
pub struct DocCmd {
    pub catalog: Arc<Catalog>,
    pub op: DocOp,
}

/// What a command answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocReply {
    /// Operations applied, nodes moved, references updated.
    pub count: usize,
    /// A navigation moved the view.
    pub moved: bool,
}

fn refused(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

// ---- the operations ---------------------------------------------------------------------------

/// Removes a node and every edge into it.
fn remove_node(g: &mut Graph, id: &str) -> bool {
    let Some(at) = g.node_index(id) else { return false };
    g.nodes.remove(at);
    for n in g.nodes.iter_mut() {
        if !n.has_target(id) {
            continue;
        }
        for p in &mut n.out {
            p.targets.retain(|t| t != id);
        }
        n.out.retain(|p| !p.targets.is_empty());
    }
    true
}

/// `node_done.node` / `elapsed.node`, and nested condition lists (`RenameNodeRefs`).
fn rename_node_refs(params: &mut Value, kind: &str, id: &str, new_id: &str) {
    if !params.is_table() {
        return;
    }
    if (kind == "node_done" || kind == "elapsed")
        && let Some(t) = params.table_mut()
        && let Some(n) = t.get_mut("node")
        && n.as_str() == Some(id)
    {
        *n = Value::from(new_id);
    }
    let Some(t) = params.table_mut() else { return };
    for (_, v) in &mut t.fields {
        let Some(vt) = v.table_mut() else { continue };
        for e in &mut vt.arr {
            if !e.is_table() {
                continue;
            }
            let ek = e.get_string("kind", "");
            if let Some(p) = e.table_mut().and_then(|t| t.get_mut("params")) {
                rename_node_refs(p, &ek, id, new_id);
            }
            if let Some(cl) = e.table_mut().and_then(|t| t.get_mut("cond")).and_then(Value::table_mut) {
                for c in &mut cl.arr {
                    let ck = c.get_string("kind", "");
                    if let Some(p2) = c.table_mut().and_then(|t| t.get_mut("params")) {
                        rename_node_refs(p2, &ck, id, new_id);
                    }
                }
            }
        }
    }
}

fn rename_node(g: &mut Graph, id: &str, new_id: &str) {
    if let Some(n) = g.find_node_mut(id) {
        n.id = SmolStr::new(new_id);
    }
    for m in g.nodes.iter_mut() {
        for p in &mut m.out {
            for t in &mut p.targets {
                if t == id {
                    *t = SmolStr::new(new_id);
                }
            }
        }
        for c in &mut m.cond {
            rename_node_refs(&mut c.params, &c.kind, id, new_id);
        }
        for a in m.on_enter.iter_mut().chain(m.on_exit.iter_mut()) {
            rename_node_refs(&mut a.params, &a.kind, id, new_id);
        }
        let kind = m.kind.clone();
        rename_node_refs(&mut m.params, &kind, id, new_id);
    }
}

/// 1-based in the protocol; absent: append (insert) or invalid (others) (`SlotIndex`).
fn slot_index(op: &Value, key: &str, size: usize, for_insert: bool) -> Option<usize> {
    let v = op.get(key).filter(|v| !v.is_nil());
    let Some(v) = v else { return if for_insert { Some(size) } else { None } };
    let i = v.to_number(0.0) as i64 - 1;
    let i = usize::try_from(i).ok()?;
    if for_insert { Some(i.min(size)) } else { (i < size).then_some(i) }
}

/// Which list `slot` names: `enter` / `on_enter` or `exit` / `on_exit` (`SlotOf`).
fn slot_exit(name: &str) -> Result<bool, String> {
    match name {
        "enter" | "on_enter" => Ok(false),
        "exit" | "on_exit" => Ok(true),
        _ => Err("slot must be \"enter\" or \"exit\"".into()),
    }
}

fn list_mut(n: &mut Node, exit: bool) -> &mut Vec<Action> {
    if exit { &mut n.on_exit } else { &mut n.on_enter }
}

fn node_field(cat: &Catalog, n: &Node, key: &str, v: &Value) -> Result<Node, String> {
    // the node reader on a minimal table
    let mut tmp = Value::table();
    tmp.set("id", n.id.clone());
    tmp.set("kind", n.kind.clone());
    tmp.set(key, v.clone());
    let mut shape = Vec::new();
    match asset::node_from_value(cat, &tmp, &mut shape) {
        Some(parsed) if shape.is_empty() => Ok(parsed),
        _ => Err(shape.into_iter().next().unwrap_or_else(|| format!("bad {key}"))),
    }
}

/// One operation of `quest_apply` (`ApplyOne`); the caller owns the undo step.
#[allow(clippy::too_many_lines)]
pub fn apply_one(cat: &Catalog, g: &mut Graph, op: &Value) -> Result<(), String> {
    if !op.is_table() {
        return Err("operation must be a table { op = \"...\", ... }".into());
    }
    let what = op.get_string("op", "");
    if what.is_empty() {
        return Err("operation without op".into());
    }
    match what.as_str() {
        "add_node" => {
            let Some(nv) = op.get("node") else {
                return Err("add_node needs node = { id = ..., kind = ..., ... }".into());
            };
            let mut shape = Vec::new();
            let n = asset::node_from_value(cat, nv, &mut shape);
            let n = match n {
                Some(n) if shape.is_empty() => n,
                _ => return Err(shape.into_iter().next().unwrap_or_else(|| "bad node".into())),
            };
            if g.find_node(&n.id).is_some() {
                return Err(format!("node '{}' already exists", n.id));
            }
            g.nodes.push_back(n);
            Ok(())
        }
        "remove_node" => {
            let id = op.get_string("id", "");
            if !remove_node(g, &id) {
                return Err(format!("node '{id}' does not exist"));
            }
            Ok(())
        }
        "rename_node" => {
            let (id, new_id) = (op.get_string("id", ""), op.get_string("new_id", ""));
            if g.find_node(&id).is_none() {
                return Err(format!("node '{id}' does not exist"));
            }
            if new_id.is_empty() {
                return Err("rename_node needs new_id".into());
            }
            if id == new_id {
                return Ok(());
            }
            if g.find_node(&new_id).is_some() {
                return Err(format!("node '{new_id}' already exists"));
            }
            rename_node(g, &id, &new_id);
            Ok(())
        }
        "set_node" => {
            let id = op.get_string("id", "");
            let Some(at) = g.node_index(&id) else { return Err(format!("node '{id}' does not exist")) };
            let Some(patch) = op.get("patch").filter(|p| p.is_table()) else {
                return Err("set_node needs patch = { ... }".into());
            };
            let Some(mut n) = g.nodes.get(at).cloned() else {
                return Err(format!("node '{id}' does not exist"));
            };
            for (k, v) in patch.fields() {
                match k.as_str() {
                    "id" => return Err("set_node cannot change id - use rename_node".into()),
                    "kind" => match v.as_str() {
                        Some(s) => n.kind = SmolStr::new(s),
                        None => return Err("kind must be a string".into()),
                    },
                    "once" => {
                        if v.as_str() == Some("default") {
                            n.once_set = false;
                        } else {
                            n.once = v.to_bool(false);
                            n.once_set = true;
                        }
                    }
                    "params" => {
                        if !v.is_table() && !v.is_nil() {
                            return Err("params must be a table".into());
                        }
                        n.params = v.clone();
                    }
                    "comment" => n.comment = v.to_text(""),
                    "pos" => {
                        if v.is_table() && v.arr().len() >= 2 {
                            let a = v.arr();
                            n.pos = [
                                a.first().map_or(0.0, |x| x.to_number(0.0)) as f32,
                                a.get(1).map_or(0.0, |x| x.to_number(0.0)) as f32,
                            ];
                            n.has_pos = true;
                        } else {
                            n.has_pos = false;
                        }
                    }
                    "cond" | "on_enter" | "on_exit" | "out" => {
                        let parsed = node_field(cat, &n, k, v)?;
                        match k.as_str() {
                            "cond" => n.cond = parsed.cond,
                            "on_enter" => n.on_enter = parsed.on_enter,
                            "on_exit" => n.on_exit = parsed.on_exit,
                            _ => n.out = parsed.out,
                        }
                    }
                    other => return Err(format!("set_node: unknown field '{other}'")),
                }
            }
            g.nodes.set(at, n);
            Ok(())
        }
        "connect" | "disconnect" => {
            let (from, pin, to) =
                (op.get_string("from", ""), op.get_string("pin", "next"), op.get_string("to", ""));
            let Some(f) = g.find_node_mut(&from) else { return Err(format!("node '{from}' does not exist")) };
            if what == "connect" {
                // a missing target or an undeclared pin is a validation problem, not a refusal
                if to.is_empty() {
                    return Err("connect needs to = <node id>".into());
                }
                f.connect(&pin, &to);
            } else if !to.is_empty() {
                f.disconnect(&pin, &to);
            } else {
                f.out.retain(|p| p.name != pin.as_str());
            }
            Ok(())
        }
        "add_action" | "set_action" | "remove_action" | "move_action" => {
            let id = op.get_string("id", "");
            let Some(ni) = g.node_index(&id) else { return Err(format!("node '{id}' does not exist")) };
            let exit = slot_exit(&op.get_string("slot", "enter"))?;
            let size = g.nodes.get(ni).map_or(0, |n| if exit { n.on_exit.len() } else { n.on_enter.len() });
            if what == "add_action" {
                let a = match op.get("action").map(asset::action_from_value) {
                    Some(Ok(a)) => a,
                    Some(Err(e)) => return Err(e),
                    None => return Err("add_action needs action = { kind = ..., params = { ... } }".into()),
                };
                let Some(at) = slot_index(op, "index", size, true) else {
                    return Err("index out of range (1-based)".into());
                };
                if let Some(n) = g.nodes.get_mut(ni) {
                    list_mut(n, exit).insert(at, a);
                }
                return Ok(());
            }
            let Some(at) = slot_index(op, "index", size, false) else {
                return Err("index out of range (1-based)".into());
            };
            if what == "remove_action" {
                if let Some(n) = g.nodes.get_mut(ni) {
                    list_mut(n, exit).remove(at);
                }
                return Ok(());
            }
            if what == "set_action" {
                let Some(patch) = op.get("patch").filter(|p| p.is_table()) else {
                    return Err("set_action needs patch = { kind = ..., params = { ... } }".into());
                };
                let Some(a) = g.nodes.get_mut(ni).and_then(|n| list_mut(n, exit).get_mut(at)) else {
                    return Err("index out of range (1-based)".into());
                };
                if let Some(k) = patch.get("kind") {
                    match k.as_str() {
                        Some(s) => a.kind = SmolStr::new(s),
                        None => return Err("kind must be a string".into()),
                    }
                }
                if let Some(p) = patch.get("params") {
                    if !p.is_table() && !p.is_nil() {
                        return Err("params must be a table".into());
                    }
                    a.params = p.clone();
                }
                return Ok(());
            }
            // move_action: to_id / to_slot / to_index, all optional
            let Some(moving) = g.nodes.get_mut(ni).map(|n| list_mut(n, exit).remove(at)) else {
                return Err("index out of range (1-based)".into());
            };
            let target = if op.has("to_id") {
                match g.node_index(&op.get_string("to_id", "")) {
                    Some(t) => t,
                    None => return Err("target node does not exist".into()),
                }
            } else {
                ni
            };
            let dst_exit = if op.has("to_slot") || target != ni {
                slot_exit(&op.get_string("to_slot", &op.get_string("slot", "enter")))?
            } else {
                exit
            };
            let dsize =
                g.nodes.get(target).map_or(0, |n| if dst_exit { n.on_exit.len() } else { n.on_enter.len() });
            let Some(to) = slot_index(op, "to_index", dsize, true) else {
                return Err("to_index out of range (1-based)".into());
            };
            if let Some(n) = g.nodes.get_mut(target) {
                list_mut(n, dst_exit).insert(to, moving);
            }
            Ok(())
        }
        "set_quest" => {
            let Some(patch) = op.get("patch").filter(|p| p.is_table()) else {
                return Err("set_quest needs patch = { id/title/activation/vars/tasks }".into());
            };
            for (k, v) in patch.fields() {
                match k.as_str() {
                    "id" => g.id = v.to_text(""),
                    "title" => g.title = v.clone(),
                    "activation" => g.activation = v.to_text(""),
                    "nq" => g.nq = v.to_number(1.0) as i32,
                    "vars" => {
                        if !v.is_table() {
                            return Err("vars must be a table".into());
                        }
                        g.vars = v
                            .fields()
                            .iter()
                            .map(|(name, value)| Var { name: name.to_string(), value: value.clone() })
                            .collect();
                    }
                    "tasks" => {
                        if !v.is_table() {
                            return Err("tasks must be a table".into());
                        }
                        g.tasks = v
                            .fields()
                            .iter()
                            .map(|(id, tv)| asset::task_from_value(id, tv))
                            .collect::<imbl::Vector<Task>>();
                    }
                    other => return Err(format!("set_quest: unknown field '{other}'")),
                }
            }
            Ok(())
        }
        "set_pos" => {
            let id = op.get_string("id", "");
            let (x, y) = (op.get_number("x", 0.0) as f32, op.get_number("y", 0.0) as f32);
            let Some(n) = g.find_node_mut(&id) else { return Err(format!("node '{id}' does not exist")) };
            n.pos = [x, y];
            n.has_pos = true;
            Ok(())
        }
        other => Err(format!("unknown op '{other}'")),
    }
}

/// A list of operations, all or nothing (`ApplyOps`); a single operation without the outer list
/// is accepted. Returns how many were applied.
pub fn apply_ops(cat: &Catalog, g: &mut Graph, ops: &Value) -> Result<usize, String> {
    if !ops.is_table() {
        return Err("ops must be a list of operations".into());
    }
    let mut work = g.clone();
    let applied = if ops.arr().is_empty() && ops.has("op") {
        apply_one(cat, &mut work, ops)?;
        1
    } else {
        for (i, op) in ops.arr().iter().enumerate() {
            apply_one(cat, &mut work, op).map_err(|e| format!("op #{}: {e}", i + 1))?;
        }
        ops.arr().len()
    };
    *g = work;
    Ok(applied)
}

/// Whether `from` may be wired to `to` (`CanLink`): the target exists, is not the source and is
/// no trigger.
pub fn can_link(cat: &Catalog, g: &Graph, from: &str, to: &str) -> Result<(), String> {
    let Some(t) = g.find_node(to) else { return Err(format!("'{to}' does not exist")) };
    if from == to {
        return Err("A node cannot link to itself".into());
    }
    if cat.is_trigger(&t.kind) {
        return Err(format!("'{}' is a trigger - triggers have no input", t.id));
    }
    Ok(())
}

fn snap(v: f32) -> f32 {
    (v / GRID + 0.5).floor() * GRID
}

/// Copies nodes into the graph with free ids, edges kept inside the fragment only, shifted by
/// `delta`; nodes without a place stack down (`InsertNodes`). Returns the new ids.
fn insert_nodes(cat: &Catalog, g: &mut Graph, src: Vec<Node>, place: [f32; 2]) -> Vec<SmolStr> {
    let mut remap: Vec<(SmolStr, SmolStr)> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut nodes = src;
    for n in &mut nodes {
        let base = if n.id.is_empty() { "node".to_owned() } else { n.id.to_string() };
        let mut nid = base.clone();
        let mut k = 2;
        while g.find_node(&nid).is_some() || taken.contains(&nid) {
            nid = format!("{base}{k}");
            k += 1;
        }
        taken.push(nid.clone());
        remap.push((n.id.clone(), SmolStr::new(&nid)));
        n.id = SmolStr::new(&nid);
    }
    let mut stack_y = place[1];
    let mut made = Vec::with_capacity(nodes.len());
    for mut n in nodes {
        for p in &mut n.out {
            p.targets = p
                .targets
                .iter()
                .filter_map(|t| remap.iter().find(|(old, _)| old == t).map(|(_, new)| new.clone()))
                .collect();
        }
        if n.has_pos {
            n.pos = [snap(n.pos[0] + place[0]), snap(n.pos[1] + place[1])];
        } else {
            n.has_pos = true;
            n.pos = [snap(place[0]), snap(stack_y)];
            stack_y += layout::node_size(cat, &n)[1] + GAP_Y;
        }
        made.push(n.id.clone());
        g.nodes.push_back(n);
    }
    made
}

/// The bridges and cuts of a bypass over `selection` (`BypassSelected`): incoming edges in graph
/// order paired with outgoing ones in pin order; what has no counterpart goes.
fn bypass(cat: &Catalog, g: &mut Graph, selection: &[SmolStr]) -> String {
    if selection.is_empty() {
        return "Select a node first".into();
    }
    let mut bridges: Vec<(SmolStr, SmolStr, SmolStr)> = Vec::new();
    let mut cuts: Vec<(SmolStr, SmolStr, SmolStr)> = Vec::new();
    let mut refused = 0;
    for mid in selection {
        let Some(n) = g.find_node(mid) else { continue };
        let mut inc: Vec<(SmolStr, SmolStr, SmolStr)> = Vec::new();
        for other in &g.nodes {
            if other.id == *mid {
                continue;
            }
            for p in &other.out {
                for t in &p.targets {
                    if t == mid {
                        inc.push((other.id.clone(), p.name.clone(), mid.clone()));
                    }
                }
            }
        }
        let out: Vec<(SmolStr, SmolStr, SmolStr)> = n
            .out
            .iter()
            .flat_map(|p| p.targets.iter().map(|t| (mid.clone(), p.name.clone(), t.clone())))
            .collect();
        if inc.is_empty() && out.is_empty() {
            continue;
        }
        for (a, b) in inc.iter().zip(&out) {
            // the gate a hand-drawn edge passes
            if can_link(cat, g, &a.0, &b.2).is_err() {
                refused += 1;
                continue;
            }
            bridges.push((a.0.clone(), a.1.clone(), b.2.clone()));
        }
        cuts.extend(inc);
        cuts.extend(out);
    }
    if cuts.is_empty() {
        return "Nothing was connected to the selection".into();
    }
    for (from, pin, to) in &cuts {
        if let Some(f) = g.find_node_mut(from) {
            f.disconnect(pin, to);
        }
    }
    for (from, pin, to) in &bridges {
        if let Some(f) = g.find_node_mut(from) {
            f.connect(pin, to);
        }
    }
    if refused > 0 {
        format!("Bypassed: {} link(s) bridged, {refused} could not be", bridges.len())
    } else if bridges.is_empty() {
        "Bypassed: the selection was wired on one side only, those links are gone".into()
    } else {
        format!("Bypassed: {} link(s) bridged", bridges.len())
    }
}

/// A node can take one more incoming edge when nothing arrives yet; `flow.join` always can.
fn can_take_input(cat: &Catalog, g: &Graph, n: &Node) -> bool {
    if cat.is_trigger(&n.kind) {
        return false;
    }
    n.kind == "flow.join" || g.in_degree(&n.id) == 0
}

fn pin_free(n: &Node, pin: &str) -> bool {
    n.targets(pin).is_none_or(<[SmolStr]>::is_empty)
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])
}

/// The selected node reaches for the nearest node it can still be wired to, either way, over
/// declared pins only (`ConnectNearest`). Returns the edge and the status line.
fn connect_nearest(
    cat: &Catalog,
    g: &Graph,
    selection: &[SmolStr],
) -> (Option<(String, String, String)>, String) {
    let [sel] = selection else { return (None, "Q wires one node - select exactly one".into()) };
    let Some(si) = g.node_index(sel) else { return (None, String::new()) };
    struct Geo {
        pos: [f32; 2],
        size: [f32; 2],
        pins: Vec<String>,
        declared: usize,
    }
    let geo: Vec<Geo> = g
        .nodes
        .iter()
        .map(|n| {
            let (pins, declared) = layout::pins(cat, n);
            Geo { pos: n.pos, size: layout::node_size(cat, n), pins, declared }
        })
        .collect();
    let (Some(sn), Some(sg)) = (g.nodes.get(si), geo.get(si)) else { return (None, String::new()) };
    let mut best = f32::MAX;
    let mut found: Option<(String, String, String)> = None;
    // out of the selection into somebody's input
    for p in 0..sg.declared {
        let Some(pin) = sg.pins.get(p) else { continue };
        if !pin_free(sn, pin) {
            continue;
        }
        let a = layout::output_pin(sg.pos, sg.size, p, sg.pins.len());
        for (i, (n, ng)) in g.nodes.iter().zip(&geo).enumerate() {
            if i == si || !can_take_input(cat, g, n) {
                continue;
            }
            let d = dist2(a, layout::input_pin(ng.pos, ng.size));
            if d < best {
                best = d;
                found = Some((sn.id.to_string(), pin.clone(), n.id.to_string()));
            }
        }
    }
    // somebody's free output into the selection
    if can_take_input(cat, g, sn) {
        let b = layout::input_pin(sg.pos, sg.size);
        for (i, (o, og)) in g.nodes.iter().zip(&geo).enumerate() {
            if i == si {
                continue;
            }
            for p in 0..og.declared {
                let Some(pin) = og.pins.get(p) else { continue };
                if !pin_free(o, pin) {
                    continue;
                }
                let d = dist2(layout::output_pin(og.pos, og.size, p, og.pins.len()), b);
                if d < best {
                    best = d;
                    found = Some((o.id.to_string(), pin.clone(), sn.id.to_string()));
                }
            }
        }
    }
    let Some((from, pin, to)) = found else { return (None, "Q found no free pin to connect".into()) };
    if let Err(why) = can_link(cat, g, &from, &to) {
        return (None, why);
    }
    let status = format!("Connected {from}.{pin} -> {to}");
    (Some((from, pin, to)), status)
}

/// The search results of a document (`RefreshFind`): what the window's Find shows.
#[must_use]
pub fn find_results(cat: &Catalog, g: &Graph, query: &str) -> Vec<FindHit> {
    let behaviour = g.is_behaviour();
    let title = |k: &str| text::kind_title(cat, k, behaviour);
    index::find_in_graph(g, &index::parse_query(query), &title)
}

impl GraphDoc {
    /// What follows every content change (`Changed`): pins in catalog order, places for nodes
    /// without one, the selection pruned.
    fn changed(&mut self, cat: &Catalog) {
        // the load's shape problems are reported until the first edit, as the C++ did
        self.shape = Arc::from(Vec::new());
        asset::canonicalize_all_pins(cat, &mut self.graph);
        layout::ensure_positions(cat, &mut self.graph);
        self.view.prune(&self.graph);
    }

    /// The index of the search's current node in `results`, -1 when none.
    #[must_use]
    pub fn find_index(&self, results: &[FindHit]) -> i64 {
        self.view
            .find_node
            .as_ref()
            .and_then(|n| results.iter().position(|r| r.node == n.as_str()))
            .map_or(-1, |i| i64::try_from(i).unwrap_or(-1))
    }

    fn view_op(&mut self, cat: &Catalog, op: ViewOp) -> Result<DocReply, CmdError> {
        let g = &self.graph;
        let v = &mut self.view;
        let mut reply = DocReply::default();
        match op {
            ViewOp::Select(ids) => {
                v.selection.clear();
                v.slot.clear();
                for id in ids {
                    if !v.selection.iter().any(|s| *s == id) {
                        v.selection.push(SmolStr::new(&id));
                    }
                }
            }
            ViewOp::Slot(s) => v.slot = if s == "none" { String::new() } else { s },
            ViewOp::Center(c) => {
                let c = [layout::sane(c[0]), layout::sane(c[1])];
                #[allow(clippy::float_cmp)]
                if c != v.center {
                    v.remember();
                }
                v.center = c;
            }
            ViewOp::Zoom(z) => {
                let z = z.clamp(0, ZOOM.len() as i32 - 1);
                if z != v.zoom {
                    v.remember();
                }
                v.zoom = z;
            }
            ViewOp::FrameNode(id) => v.frame_node(cat, g, &id),
            ViewOp::HistoryBack => {
                if let Some(target) = v.back.pop() {
                    let cur = v.current();
                    v.forward.push(cur);
                    if v.forward.len() > HISTORY_LIMIT {
                        v.forward.remove(0);
                    }
                    v.apply_state(&target, g);
                    reply.moved = true;
                }
            }
            ViewOp::HistoryForward => {
                if let Some(target) = v.forward.pop() {
                    let cur = v.current();
                    v.back.push(cur);
                    if v.back.len() > HISTORY_LIMIT {
                        v.back.remove(0);
                    }
                    v.apply_state(&target, g);
                    reply.moved = true;
                }
            }
            ViewOp::HistoryClear => {
                v.back.clear();
                v.forward.clear();
            }
            ViewOp::Bookmark { node, on } => {
                if node.is_empty() {
                    return Err(refused("bookmark update failed"));
                }
                match v.bookmarks.iter().position(|b| *b == node) {
                    Some(at) if !on => {
                        v.bookmarks.remove(at);
                    }
                    Some(_) => {}
                    None => {
                        if g.find_node(&node).is_none() {
                            return Err(refused("bookmark update failed"));
                        }
                        if on {
                            v.bookmarks.push(SmolStr::new(&node));
                        }
                    }
                }
            }
            ViewOp::BookmarksClear => v.bookmarks.clear(),
            ViewOp::BookmarkJump(delta) => {
                v.bookmarks.retain(|b| g.find_node(b).is_some());
                if v.bookmarks.is_empty() {
                    return Err(refused("no bookmarks"));
                }
                let len = v.bookmarks.len() as i64;
                let current = match v.selection.as_slice() {
                    [one] => v.bookmarks.iter().position(|b| b == one).map_or(-1, |i| i as i64),
                    _ => -1,
                };
                let mut next =
                    if delta < 0 { if current < 0 { len - 1 } else { current - 1 } } else { current + 1 };
                if next < 0 {
                    next = len - 1;
                }
                if next >= len {
                    next = 0;
                }
                let id = v.bookmarks.get(usize::try_from(next).unwrap_or(0)).cloned().unwrap_or_default();
                v.frame_node(cat, g, &id);
            }
            ViewOp::FindQuery(q) => {
                v.find_query = q;
                v.drawer = SmolStr::new("find");
                // the current node stays when the new query still finds it
                let results = find_results(cat, g, &v.find_query);
                if v.find_node.as_ref().is_some_and(|n| !results.iter().any(|r| r.node == n.as_str())) {
                    v.find_node = None;
                }
            }
            ViewOp::FindGo(index) => {
                let results = find_results(cat, g, &v.find_query);
                if results.is_empty() {
                    v.find_node = None;
                    return Ok(reply);
                }
                let len = results.len() as i64;
                let mut i = index;
                if i < 0 {
                    i = len - 1;
                }
                if i >= len {
                    i = 0;
                }
                let node =
                    results.get(usize::try_from(i).unwrap_or(0)).map(|r| r.node.clone()).unwrap_or_default();
                v.find_node = Some(SmolStr::new(&node));
                v.frame_node(cat, g, &node);
                reply.moved = true;
            }
            ViewOp::FindClear => {
                v.find_query.clear();
                v.find_node = None;
                if v.drawer == "find" {
                    v.drawer = SmolStr::new("none");
                }
            }
            ViewOp::Minimap(on) => v.minimap = on,
            ViewOp::Drawn { canvas, dpi, minimap_shown } => {
                v.canvas = canvas;
                v.dpi = dpi;
                v.minimap_shown = minimap_shown;
            }
            ViewOp::Drawer(d) => v.drawer = d,
        }
        Ok(reply)
    }
}

impl State for GraphDoc {
    type Cmd = DocCmd;
    type Reply = DocReply;

    #[allow(clippy::too_many_lines)]
    fn apply(&mut self, cmd: DocCmd, cx: &mut Apply) -> Result<DocReply, CmdError> {
        let cat = &*cmd.catalog;
        let mut reply = DocReply::default();
        match cmd.op {
            DocOp::Load { graph, shape, file_time } => {
                self.graph = graph;
                // nodes without coordinates get a place right away
                layout::ensure_positions(cat, &mut self.graph);
                self.partial = !shape.is_empty();
                self.shape = Arc::from(shape);
                self.file_time = file_time;
                self.view.prune(&self.graph);
                cx.new_document();
            }
            DocOp::Replace(graph) => {
                self.graph = graph;
                self.partial = false;
                self.shape = Arc::from(Vec::new());
                self.changed(cat);
                cx.undoable("Replace Text");
            }
            DocOp::ApplyOps(ops) => {
                reply.count = apply_ops(cat, &mut self.graph, &ops).map_err(refused)?;
                if reply.count == 0 {
                    cx.unchanged();
                    return Ok(reply);
                }
                self.changed(cat);
                cx.undoable("Apply Operations");
            }
            DocOp::Apply(op) => {
                let mut work = self.graph.clone();
                apply_one(cat, &mut work, &op).map_err(refused)?;
                self.graph = work;
                self.changed(cat);
                cx.undoable("Edit Graph");
                reply.count = 1;
            }
            DocOp::AddNode(n) => {
                if n.id.is_empty() {
                    return Err(refused("node id is empty"));
                }
                if self.graph.find_node(&n.id).is_some() {
                    return Err(refused(format!("node '{}' already exists", n.id)));
                }
                self.graph.nodes.push_back(n);
                self.changed(cat);
                cx.undoable("Add Node");
            }
            DocOp::RemoveNodes(ids) => {
                if let Some(missing) = ids.iter().find(|id| self.graph.find_node(id).is_none()) {
                    return Err(refused(format!("node '{missing}' does not exist")));
                }
                if ids.is_empty() {
                    cx.unchanged();
                    return Ok(reply);
                }
                for id in &ids {
                    remove_node(&mut self.graph, id);
                }
                self.changed(cat);
                cx.undoable(if ids.len() == 1 { "Delete Node" } else { "Delete Nodes" });
            }
            DocOp::RenameNode { id, new_id } => {
                if self.graph.find_node(&id).is_none() {
                    return Err(refused("node does not exist"));
                }
                if new_id.is_empty() {
                    return Err(refused("new id is empty"));
                }
                if id == new_id {
                    cx.unchanged();
                    return Ok(reply);
                }
                if self.graph.find_node(&new_id).is_some() {
                    return Err(refused(format!("node '{new_id}' already exists")));
                }
                rename_node(&mut self.graph, &id, &new_id);
                for s in &mut self.view.selection {
                    if *s == id {
                        *s = SmolStr::new(&new_id);
                    }
                }
                self.changed(cat);
                cx.undoable("Rename Node");
            }
            DocOp::RenameTask { from, to, path } => {
                if from.is_empty() {
                    return Err(refused("task id is empty"));
                }
                if to.is_empty() {
                    return Err(refused("new task id is empty"));
                }
                if !text::valid_id(&to) {
                    return Err(refused("new task id must be [a-z0-9_]+"));
                }
                let declarations = self.graph.tasks.iter().filter(|t| t.id == from).count();
                if declarations == 0 {
                    return Err(refused(format!("task '{from}' does not exist")));
                }
                if declarations != 1 {
                    return Err(refused(format!(
                        "task '{from}' is declared {declarations} times; rename requires exactly one source declaration"
                    )));
                }
                if from == to {
                    return Err(refused("new task id is unchanged"));
                }
                if self.graph.find_task(&to).is_some() {
                    return Err(refused(format!("task '{to}' already exists")));
                }
                if self.partial {
                    return Err(refused(
                        "task rename refused: the open document is an incomplete model of a malformed source file",
                    ));
                }
                let (after, updated) =
                    references::rename_task(&self.graph, cat, &path, &from, &to).map_err(|diags| {
                        let mut err =
                            String::from("task rename refused because reference coverage is incomplete");
                        for (i, d) in diags.iter().enumerate() {
                            err.push_str(if i == 0 { ": " } else { "; " });
                            err.push_str(&d.text());
                        }
                        refused(err)
                    })?;
                self.graph = after;
                reply.count = updated;
                self.changed(cat);
                cx.undoable("Rename Task");
            }
            DocOp::Connect { from, pin, to } => {
                if self.graph.find_node(&from).is_none() {
                    return Err(refused("source node does not exist"));
                }
                if to.is_empty() {
                    return Err(refused("target id is empty"));
                }
                if pin.is_empty() {
                    return Err(refused("pin is empty"));
                }
                if let Some(f) = self.graph.find_node_mut(&from) {
                    f.connect(&pin, &to);
                }
                self.changed(cat);
                cx.undoable("Connect");
            }
            DocOp::Disconnect { from, pin, to } => {
                let Some(f) = self.graph.find_node_mut(&from) else {
                    return Err(refused("source node does not exist"));
                };
                if to.is_empty() {
                    f.out.retain(|p| !(pin.is_empty() || p.name == pin.as_str()));
                } else {
                    f.disconnect(&pin, &to);
                }
                self.changed(cat);
                cx.undoable("Disconnect");
            }
            DocOp::SetPos { id, x, y, undo } => {
                let Some(n) = self.graph.find_node_mut(&id) else {
                    return Err(refused(format!("node '{id}' does not exist")));
                };
                // a drag never goes through the change pass, so the bound is applied here too
                n.pos = [layout::sane(x), layout::sane(y)];
                n.has_pos = true;
                if undo {
                    cx.undoable("Move Node");
                } else {
                    cx.content_changed();
                }
            }
            DocOp::Layout { only_missing } => {
                let moved = layout::run(cat, &mut self.graph, only_missing);
                reply.count = moved;
                if moved == 0 {
                    cx.unchanged();
                    return Ok(reply);
                }
                self.changed(cat);
                cx.undoable("Auto Layout");
            }
            DocOp::Canvas(op) => {
                let selection = self.view.selection.clone();
                match op {
                    CanvasOp::SelectAll => {
                        self.view.selection = self.graph.nodes.iter().map(|n| n.id.clone()).collect();
                    }
                    CanvasOp::Delete => {
                        if selection.is_empty() {
                            cx.unchanged();
                            return Ok(reply);
                        }
                        if selection.iter().any(|id| self.graph.find_node(id).is_none()) {
                            cx.unchanged();
                            return Ok(reply);
                        }
                        for id in &selection {
                            remove_node(&mut self.graph, id);
                        }
                        self.view.selection.clear();
                        self.view.slot.clear();
                        self.changed(cat);
                        cx.undoable(if selection.len() == 1 { "Delete Node" } else { "Delete Nodes" });
                    }
                    CanvasOp::Duplicate => {
                        if selection.is_empty() {
                            cx.unchanged();
                            return Ok(reply);
                        }
                        let src: Vec<Node> =
                            selection.iter().filter_map(|id| self.graph.find_node(id).cloned()).collect();
                        let made = insert_nodes(cat, &mut self.graph, src, [GRID * 2.0, GRID * 2.0]);
                        self.view.selection = made;
                        self.view.slot.clear();
                        self.changed(cat);
                        cx.undoable("Duplicate");
                    }
                    CanvasOp::Bypass => {
                        let before = self.graph.clone();
                        self.view.status = bypass(cat, &mut self.graph, &selection);
                        if self.graph == before {
                            return Ok(reply);
                        }
                        self.changed(cat);
                        cx.undoable("Bypass");
                    }
                    CanvasOp::ConnectNearest => {
                        let (edge, status) = connect_nearest(cat, &self.graph, &selection);
                        self.view.status = status;
                        let Some((from, pin, to)) = edge else { return Ok(reply) };
                        if let Some(f) = self.graph.find_node_mut(&from) {
                            f.connect(&pin, &to);
                        }
                        self.changed(cat);
                        cx.undoable("Connect");
                    }
                }
            }
            DocOp::View(op) => return self.view_op(cat, op),
            DocOp::Saved { file_time } => {
                self.file_time = file_time;
                cx.mark_saved();
            }
        }
        Ok(reply)
    }

    /// Undo and Redo bring the content back and keep the view, pruned to what exists.
    fn restored(&self, current: &Self) -> Self {
        let mut s = self.clone();
        s.view = current.view.clone();
        s.view.prune(&s.graph);
        s.file_time = current.file_time;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        use crate::catalog::{Kind, USE_MAIN, USE_TRIGGER};
        let kind = |id: &str, uses: u8| Kind {
            id: id.into(),
            uses,
            pins: vec!["next".into()],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        Catalog::from_kinds(vec![
            kind("trigger.start", USE_TRIGGER),
            kind("flow.step", USE_MAIN),
            kind("flow.end", USE_MAIN),
        ])
    }

    fn graph() -> Graph {
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "b");
        let mut b = Node::new("b", "flow.step");
        b.params.set("node", "a");
        let mut cond = Node::new("c", "flow.step");
        cond.cond.push(asset::Cond {
            kind: "node_done".into(),
            params: {
                let mut p = Value::table();
                p.set("node", "a");
                p
            },
            negate: false,
        });
        for n in [s, a, b, cond] {
            g.nodes.push_back(n);
        }
        g
    }

    fn ops(text: &str) -> Value {
        match gz_luadata::parse(&format!("return {text}")) {
            Ok(v) => v,
            Err(e) => panic!("{}", e.message),
        }
    }

    #[test]
    fn every_operation() {
        let c = cat();
        let mut g = graph();
        let applied = apply_ops(
            &c,
            &mut g,
            &ops(r#"{
                { op = "add_node", node = { id = "d", kind = "flow.end" } },
                { op = "connect", from = "b", to = "d" },
                { op = "rename_node", id = "a", new_id = "aa" },
                { op = "set_node", id = "b", patch = { once = true, comment = "hi", pos = { 16, 32 } } },
                { op = "add_action", id = "b", slot = "exit", action = { kind = "var.add", params = { name = "n", delta = 1 } } },
                { op = "add_action", id = "b", slot = "exit", index = 1, action = { kind = "money.give" } },
                { op = "set_action", id = "b", slot = "exit", index = 2, patch = { kind = "var.set" } },
                { op = "move_action", id = "b", slot = "exit", index = 1, to_id = "d", to_slot = "enter" },
                { op = "set_quest", patch = { id = "q2", vars = { n = 0 } } },
                { op = "set_pos", id = "d", x = 5, y = 6 },
            }"#),
        );
        assert_eq!(applied, Ok(10));
        assert_eq!(g.id, "q2");
        assert!(g.find_node("a").is_none());
        assert_eq!(
            g.find_node("s").and_then(|n| n.targets("next")).map(<[SmolStr]>::to_vec),
            Some(vec![SmolStr::new("aa")])
        );
        // a node reference inside a condition follows the rename
        assert_eq!(
            g.find_node("c").and_then(|n| n.cond.first()).map(|c| c.params.get_string("node", "")),
            Some("aa".to_owned())
        );
        let b = g.find_node("b").cloned().unwrap_or_default();
        assert!(b.once_set && b.once);
        assert_eq!((b.comment.as_str(), b.pos, b.on_exit.len()), ("hi", [16.0, 32.0], 1));
        assert_eq!(b.on_exit.first().map(|a| a.kind.as_str()), Some("var.set"));
        let d = g.find_node("d").cloned().unwrap_or_default();
        assert_eq!(d.on_enter.first().map(|a| a.kind.as_str()), Some("money.give"));
        assert_eq!(d.pos, [5.0, 6.0]);
        assert_eq!(g.find_var("n").map(|v| v.value.clone()), Some(Value::Number(0.0)));
    }

    #[test]
    fn operations_are_all_or_nothing() {
        let c = cat();
        let mut g = graph();
        let before = g.clone();
        let r = apply_ops(
            &c,
            &mut g,
            &ops(r#"{ { op = "remove_node", id = "a" }, { op = "remove_node", id = "zz" } }"#),
        );
        assert_eq!(r, Err("op #2: node 'zz' does not exist".to_owned()));
        assert_eq!(g, before);
        assert_eq!(apply_one(&c, &mut g, &ops(r#"{ op = "warp" }"#)), Err("unknown op 'warp'".to_owned()));
        assert_eq!(
            apply_one(&c, &mut g, &ops(r#"{ op = "set_node", id = "a", patch = { id = "x" } }"#)),
            Err("set_node cannot change id - use rename_node".to_owned())
        );
        assert_eq!(
            apply_one(
                &c,
                &mut g,
                &ops(r#"{ op = "add_action", id = "a", index = 9, action = { kind = "x" } }"#)
            ),
            Ok(())
        );
        assert_eq!(
            apply_one(&c, &mut g, &ops(r#"{ op = "remove_action", id = "a", index = 5 }"#)),
            Err("index out of range (1-based)".to_owned())
        );
        // removing a node takes the edges into it
        assert_eq!(apply_one(&c, &mut g, &ops(r#"{ op = "remove_node", id = "a" }"#)), Ok(()));
        assert!(g.find_node("s").is_some_and(|n| n.out.is_empty()));
    }

    #[test]
    fn canvas_operations() {
        let c = cat();
        let mut g = graph();
        assert_eq!(can_link(&c, &g, "a", "s"), Err("'s' is a trigger - triggers have no input".to_owned()));
        assert_eq!(can_link(&c, &g, "a", "a"), Err("A node cannot link to itself".to_owned()));
        let status = bypass(&c, &mut g, &[SmolStr::new("a")]);
        assert_eq!(status, "Bypassed: 1 link(s) bridged");
        assert_eq!(
            g.find_node("s").and_then(|n| n.targets("next")).map(<[SmolStr]>::to_vec),
            Some(vec![SmolStr::new("b")])
        );
        assert!(g.find_node("a").is_some_and(|n| n.out.is_empty()));
        layout::run(&c, &mut g, false);
        let copy = g.find_node("b").cloned().unwrap_or_default();
        let made = insert_nodes(&c, &mut g, vec![copy], [32.0, 32.0]);
        assert_eq!(made, [SmolStr::new("b2")]);
        let (edge, status) = connect_nearest(&c, &g, &[SmolStr::new("a")]);
        assert!(edge.is_some(), "{status}");
    }

    #[test]
    fn undo_takes_an_edit_back() {
        let rt = match gz_task::Runtime::start(gz_task::ExecOptions::default()) {
            Ok(rt) => rt,
            Err(e) => panic!("{e}"),
        };
        let exec = rt.exec();
        rt.block_on(async {
            let store = gz_store::Store::spawn(
                "doc",
                GraphDoc::default(),
                &exec,
                gz_store::StoreOptions { history_limit: 200 },
            );
            let catalog = Arc::new(cat());
            let send = |op: DocOp| store.send(DocCmd { catalog: catalog.clone(), op }, gz_base::Origin::Mcp);
            assert!(send(DocOp::Load { graph: graph(), shape: Vec::new(), file_time: None }).await.is_ok());
            assert!(!store.is_dirty());
            assert!(send(DocOp::RemoveNodes(vec!["b".into()])).await.is_ok());
            assert!(store.is_dirty());
            assert!(store.snapshot().graph.find_node("b").is_none());
            assert_eq!(store.history().undo, ["Delete Node"]);
            assert_eq!(store.undo().await.as_deref(), Some("Delete Node"));
            assert!(store.snapshot().graph.find_node("b").is_some());
            assert!(!store.is_dirty());
            assert_eq!(store.redo().await.as_deref(), Some("Delete Node"));
            assert!(store.snapshot().graph.find_node("b").is_none());
            // a refused edit changes nothing and records no step
            assert!(send(DocOp::RenameNode { id: "zz".into(), new_id: "y".into() }).await.is_err());
            assert_eq!(store.history().undo, ["Delete Node"]);
            // the view follows: a selection of a node that went is pruned
            assert!(send(DocOp::View(ViewOp::Select(vec!["s".into(), "a".into()]))).await.is_ok());
            assert!(send(DocOp::Canvas(CanvasOp::Delete)).await.is_ok());
            assert!(store.snapshot().view.selection.is_empty());
            assert!(store.snapshot().graph.find_node("a").is_none());
        });
    }
}
