//! An open document: the state of one `Store<GraphDoc>` and every edit `graph.edit`,
//! `graph.arrange` and the canvas make, as commands of that store.
//!
//! A command carries the catalog it is applied with, because what a node looks like (its pins
//! in order, its box, whether it is a trigger) is the catalog's. Undo, redo and dirtiness are
//! the store's; the view state (selection, the Details slot, the view's centre and zoom, the
//! view history, the search) rides along and Undo keeps it. What the file keeps for the editor -
//! frames, routes, bookmarks, the flow, the wire options - is content: an edit of it is one undo
//! step and makes the document dirty.

use std::sync::Arc;
use std::time::SystemTime;

use gz_luadata::Value;
use gz_store::{Apply, CmdError, State};
use smol_str::SmolStr;

use crate::arrange::{self, Align, Scope};
use crate::asset::{self, Action, Graph, Node, Task, Var};
use crate::canvas::{Bookmark, Diagonal, Flow, Frame, WireStyle};
use crate::catalog::Catalog;
use crate::index::{self, FindHit};
use crate::layout::{self, BoxCx, GRID};
use crate::references;
use crate::text;

/// The canvas's zoom levels, 5% to 200%; [`ZOOM_DEFAULT`] is 1:1.
pub const ZOOM: [f32; 21] = [
    0.05, 0.06, 0.075, 0.09, 0.11, 0.13, 0.16, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0, 1.25,
    1.5, 1.75, 2.0,
];
pub const ZOOM_DEFAULT: i32 = 16;
/// Views kept by the view history.
pub const HISTORY_LIMIT: usize = 64;
/// A graph of this many nodes gets a minimap.
pub const MINIMAP_MIN_NODES: usize = 12;

/// The factor of a zoom level.
#[must_use]
pub fn zoom_of(level: i32) -> f32 {
    let i = usize::try_from(level.clamp(0, ZOOM.len() as i32 - 1)).unwrap_or(16);
    ZOOM.get(i).copied().unwrap_or(1.0)
}

/// The highest level whose factor is no larger than `zoom`.
#[must_use]
pub fn level_at_most(zoom: f32) -> i32 {
    ZOOM.iter().rposition(|z| *z <= zoom + 1e-6).map_or(0, |i| i as i32)
}

/// One remembered view: where the canvas looked and what was selected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub center: [f32; 2],
    pub zoom: i32,
    pub selection: Vec<SmolStr>,
    pub slot: String,
}

/// What a document's tab shows, kept with the document.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub selection: Vec<SmolStr>,
    /// `enter:2` / `exit:0` / empty: the action the Details panel has open.
    pub slot: String,
    pub center: [f32; 2],
    pub zoom: i32,
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

    /// Pushes the current view on the back history unless it is the last one.
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
    /// one.
    fn prune(&mut self, g: &Graph) {
        self.selection.retain(|id| g.find_node(id).is_some());
        if self.selection.len() != 1 {
            self.slot.clear();
        }
    }

    fn select_only(&mut self, id: &str) {
        self.selection.clear();
        if !id.is_empty() {
            self.selection.push(SmolStr::new(id));
        }
        self.slot.clear();
    }

    /// Centres a node at 1:1 or closer and selects it.
    fn frame_node(&mut self, cat: &Catalog, g: &Graph, id: &str) {
        let Some(n) = g.find_node(id) else { return };
        let s = layout::node_size(&BoxCx::of(cat, g), n);
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
    /// The nodes renamed since the file was read or saved, in order: the save renames them in
    /// the file first, so a node keeps its comments and the keys the editor does not know.
    pub renames: Vec<(SmolStr, SmolStr)>,
}

impl Default for GraphDoc {
    fn default() -> Self {
        Self {
            graph: Graph::default(),
            shape: Arc::from(Vec::new()),
            partial: false,
            file_time: None,
            view: View::default(),
            renames: Vec::new(),
        }
    }
}

/// A canvas edit on the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasOp {
    /// Closes the chain over the selection: A to B to C becomes A to C.
    Bypass,
    Delete,
    /// `wires`: the copies keep the wires into the originals.
    Duplicate {
        wires: bool,
    },
    ConnectNearest,
    SelectAll,
}

/// Which way a selection grows along the wires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grow {
    /// Both ways, every node a wire reaches.
    Connected,
    Upstream,
    Downstream,
}

/// A view change: navigation, the history, the search, the minimap.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewOp {
    Select(Vec<String>),
    /// The selection grows along the wires.
    Grow(Grow),
    /// `none` or empty closes the action.
    Slot(String),
    Center([f32; 2]),
    Zoom(i32),
    FrameNode(String),
    HistoryBack,
    HistoryForward,
    HistoryClear,
    /// +1 or -1 through the bookmarks.
    BookmarkJump(i32),
    /// To a bookmark by its name, else by its number (1-based).
    BookmarkGo(String),
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

/// What `graph.arrange` does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrangeAction {
    Layout,
    Align(Align),
    Distribute { horizontal: bool },
    Straighten,
}

/// One arrangement: what, of which nodes (none: the selection), how far, in which flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrange {
    pub action: ArrangeAction,
    pub nodes: Vec<String>,
    pub scope: Scope,
    /// Switches the graph's flow first (with `Layout`).
    pub flow: Option<Flow>,
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
    /// The whole graph replaced, one undo step.
    Replace(Graph),
    /// A list of operations, all or nothing, one undo step.
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
    /// Several nodes moved at once (a drag, an arrangement): one undo step named `label`.
    Positions {
        at: Vec<(SmolStr, [f32; 2])>,
        label: &'static str,
    },
    Layout {
        only_missing: bool,
    },
    Arrange(Arrange),
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

/// Removes a node, every edge into it and what the canvas keeps of it.
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
    g.canvas.forget_node(id);
    true
}

/// `node_done.node` / `elapsed.node`, and nested condition lists.
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
    g.canvas.rename_node(id, new_id);
}

/// 1-based in the protocol; absent: append (insert) or invalid (others).
fn slot_index(op: &Value, key: &str, size: usize, for_insert: bool) -> Option<usize> {
    let v = op.get(key).filter(|v| !v.is_nil());
    let Some(v) = v else { return if for_insert { Some(size) } else { None } };
    let i = v.to_number(0.0) as i64 - 1;
    let i = usize::try_from(i).ok()?;
    if for_insert { Some(i.min(size)) } else { (i < size).then_some(i) }
}

/// Which list `slot` names: `enter` / `on_enter` or `exit` / `on_exit`.
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

fn snap(v: f32) -> f32 {
    layout::snap(v)
}

fn point_of(v: &Value) -> Option<[f32; 2]> {
    let a = v.arr();
    if a.len() >= 2 {
        return Some([a.first()?.to_number(0.0) as f32, a.get(1)?.to_number(0.0) as f32]);
    }
    if v.has("x") && v.has("y") {
        return Some([v.get_number("x", 0.0) as f32, v.get_number("y", 0.0) as f32]);
    }
    None
}

/// The node an op names: `node` or `id`.
fn op_node(op: &Value) -> String {
    let n = op.get_string("node", "");
    if n.is_empty() { op.get_string("id", "") } else { n }
}

/// The nodes an op names: `nodes`, `node` or `id`; none named is the selection.
fn op_nodes(op: &Value, sel: &[SmolStr]) -> Vec<SmolStr> {
    if let Some(list) = op.get("nodes").filter(|v| v.is_table()) {
        return list.arr().iter().filter_map(Value::as_str).map(SmolStr::new).collect();
    }
    if let Some(s) = op.get("nodes").and_then(Value::as_str) {
        return s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(SmolStr::new).collect();
    }
    let one = op_node(op);
    if !one.is_empty() {
        return vec![SmolStr::new(one)];
    }
    sel.to_vec()
}

fn need_nodes(g: &Graph, ids: &[SmolStr]) -> Result<(), String> {
    if ids.is_empty() {
        return Err("no node named and nothing selected".into());
    }
    match ids.iter().find(|id| g.find_node(id).is_none()) {
        Some(missing) => Err(format!("node '{missing}' does not exist")),
        None => Ok(()),
    }
}

/// What an operation did besides the graph: the selection it leaves, the status it says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Done {
    pub select: Option<Vec<SmolStr>>,
    pub status: String,
    /// Nodes renamed: the old id, the new.
    pub renamed: Vec<(SmolStr, SmolStr)>,
}

impl Done {
    fn select(ids: Vec<SmolStr>) -> Self {
        Self { select: Some(ids), ..Self::default() }
    }

    fn status(s: impl Into<String>) -> Self {
        Self { status: s.into(), ..Self::default() }
    }
}

/// What an undo step of an op is called.
#[must_use]
pub fn label_of(op: &str) -> &'static str {
    match op {
        "add_node" | "add" => "Add Node",
        "remove_node" | "remove" => "Delete Node",
        "rename_node" | "rename" => "Rename Node",
        "set_node" | "set" => "Edit Node",
        "connect" => "Connect",
        "disconnect" | "cut" => "Cut Wires",
        "set_pos" | "move" => "Move Node",
        "replace" => "Replace Node",
        "insert_on_wire" => "Insert on Wire",
        "add_action" => "Add Action",
        "set_action" => "Edit Action",
        "remove_action" => "Remove Action",
        "move_action" => "Move Action",
        "set_quest" | "set_graph" => "Edit Graph",
        "set_task" => "Edit Task",
        "set_var" => "Edit Variable",
        "frame" => "Edit Frame",
        "route" => "Edit Reroute",
        "bookmark" => "Bookmark",
        "disable" => "Disable",
        "collapse" => "Collapse",
        "duplicate" => "Duplicate",
        "bypass" => "Bypass",
        "connect_nearest" => "Connect",
        "wires" => "Wire Style",
        "flow" => "Flow Direction",
        "paste" => "Paste",
        _ => "Edit Graph",
    }
}

/// The ids of a fragment made free against the graph and each other; the old id, the new.
fn fresh_ids(g: &Graph, nodes: &mut [Node]) -> Vec<(SmolStr, SmolStr)> {
    let mut remap: Vec<(SmolStr, SmolStr)> = Vec::with_capacity(nodes.len());
    for n in nodes.iter_mut() {
        let base = if n.id.is_empty() { "node".to_owned() } else { n.id.to_string() };
        let mut nid = base.clone();
        let mut k = 2;
        while g.find_node(&nid).is_some() || remap.iter().any(|(_, t)| t == nid.as_str()) {
            nid = format!("{base}{k}");
            k += 1;
        }
        remap.push((n.id.clone(), SmolStr::new(&nid)));
        n.id = SmolStr::new(&nid);
    }
    remap
}

/// Copies nodes into the graph with free ids, the edges kept inside the fragment, shifted by
/// `delta`; with `wires` the wires into the originals from outside lead into the copies too.
fn insert_nodes(cat: &Catalog, g: &mut Graph, src: Vec<Node>, delta: [f32; 2], wires: bool) -> Vec<SmolStr> {
    let originals: Vec<SmolStr> = src.iter().map(|n| n.id.clone()).collect();
    let mut nodes = src;
    let remap = fresh_ids(g, &mut nodes);
    let cx = BoxCx::of(cat, g);
    let mut stack_y = delta[1];
    let mut made = Vec::with_capacity(nodes.len());
    for mut n in nodes {
        for p in &mut n.out {
            p.targets = p
                .targets
                .iter()
                .filter_map(|t| remap.iter().find(|(old, _)| old == t).map(|(_, new)| new.clone()))
                .collect();
        }
        n.out.retain(|p| !p.targets.is_empty());
        if n.has_pos {
            n.pos = [snap(n.pos[0] + delta[0]), snap(n.pos[1] + delta[1])];
        } else {
            n.has_pos = true;
            n.pos = [snap(delta[0]), snap(stack_y)];
            stack_y += layout::node_size(&cx, &n)[1] + arrange::gaps(cx.flow).depth;
        }
        made.push(n.id.clone());
        g.nodes.push_back(n);
    }
    if wires {
        let mut adds: Vec<(SmolStr, SmolStr, SmolStr)> = Vec::new();
        for other in &g.nodes {
            if originals.contains(&other.id) || made.contains(&other.id) {
                continue;
            }
            for p in &other.out {
                for t in &p.targets {
                    if let Some((_, new)) = remap.iter().find(|(old, _)| old == t) {
                        adds.push((other.id.clone(), p.name.clone(), new.clone()));
                    }
                }
            }
        }
        for (from, pin, to) in adds {
            if let Some(f) = g.find_node_mut(&from) {
                f.connect(&pin, &to);
            }
        }
    }
    made
}

/// The kind of an op's node: `kind`, or the `node` table's.
fn new_node(cat: &Catalog, g: &Graph, op: &Value) -> Result<Node, String> {
    if let Some(nv) = op.get("node").filter(|v| v.is_table()) {
        let mut shape = Vec::new();
        return match asset::node_from_value(cat, nv, &mut shape) {
            Some(n) if shape.is_empty() => Ok(n),
            _ => Err(shape.into_iter().next().unwrap_or_else(|| "bad node".into())),
        };
    }
    let kind = op.get_string("kind", "");
    if kind.is_empty() {
        return Err("give the new node's kind (kind=...) or its table (node={...})".into());
    }
    let last = kind.rsplit('.').next().unwrap_or(&kind);
    let base = if last == "start" || last == "when" { "trigger" } else { last };
    let mut id = op.get_string("id", "");
    if id.is_empty() {
        id = g.free_node_id(base);
    }
    let mut n = Node::new(&id, &kind);
    n.params = Value::table();
    Ok(n)
}

/// One operation of `graph.edit`; the caller owns the undo step.
#[allow(clippy::too_many_lines)]
pub fn apply_one(cat: &Catalog, g: &mut Graph, op: &Value, sel: &[SmolStr]) -> Result<Done, String> {
    if !op.is_table() {
        return Err("operation must be a table { op = \"...\", ... }".into());
    }
    let what = op.get_string("op", "");
    if what.is_empty() {
        return Err("operation without op".into());
    }
    match what.as_str() {
        "add_node" | "add" => {
            let mut n = new_node(cat, g, op)?;
            if g.find_node(&n.id).is_some() {
                return Err(format!("node '{}' already exists", n.id));
            }
            if let Some(at) = op.get("at").and_then(point_of) {
                n.pos = [snap(at[0]), snap(at[1])];
                n.has_pos = true;
            }
            let id = n.id.clone();
            g.nodes.push_back(n);
            Ok(Done::select(vec![id]))
        }
        "remove_node" | "remove" => {
            let ids = op_nodes(op, sel);
            need_nodes(g, &ids)?;
            for id in &ids {
                remove_node(g, id);
            }
            Ok(Done::select(Vec::new()))
        }
        "rename_node" | "rename" => {
            let id = op_node(op);
            let new_id = {
                let t = op.get_string("to", "");
                if t.is_empty() { op.get_string("new_id", "") } else { t }
            };
            if g.find_node(&id).is_none() {
                return Err(format!("node '{id}' does not exist"));
            }
            if new_id.is_empty() {
                return Err("rename needs the new id (to=...)".into());
            }
            if !text::valid_id(&new_id) {
                return Err(format!("'{new_id}' is no node id: lower case letters, digits and _"));
            }
            if id == new_id {
                return Ok(Done::default());
            }
            if g.find_node(&new_id).is_some() {
                return Err(format!("node '{new_id}' already exists"));
            }
            rename_node(g, &id, &new_id);
            let mut done = Done::select(vec![SmolStr::new(&new_id)]);
            done.renamed.push((SmolStr::new(id), SmolStr::new(new_id)));
            Ok(done)
        }
        "set_node" | "set" => {
            let id = op_node(op);
            let Some(at) = g.node_index(&id) else { return Err(format!("node '{id}' does not exist")) };
            let Some(patch) = op.get("patch").filter(|p| p.is_table()) else {
                return Err("set needs patch = { ... }".into());
            };
            let Some(mut n) = g.nodes.get(at).cloned() else {
                return Err(format!("node '{id}' does not exist"));
            };
            for (k, v) in patch.fields() {
                match k.as_str() {
                    "id" => return Err("set cannot change the id - rename does".into()),
                    "kind" => match v.as_str() {
                        Some(s) => n.kind = SmolStr::new(s),
                        None => return Err("kind must be a string".into()),
                    },
                    "once" => {
                        if v.as_str() == Some("default") || v.is_nil() {
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
                    "collapsed" => n.collapsed = v.to_bool(false),
                    "disabled" => n.disabled = v.to_bool(false),
                    "pos" => match point_of(v) {
                        Some(p) => {
                            n.pos = p;
                            n.has_pos = true;
                        }
                        None => n.has_pos = false,
                    },
                    "cond" | "on_enter" | "on_exit" | "out" => {
                        let parsed = node_field(cat, &n, k, v)?;
                        match k.as_str() {
                            "cond" => n.cond = parsed.cond,
                            "on_enter" => n.on_enter = parsed.on_enter,
                            "on_exit" => n.on_exit = parsed.on_exit,
                            _ => n.out = parsed.out,
                        }
                    }
                    other => return Err(format!("set: unknown field '{other}'")),
                }
            }
            g.nodes.set(at, n);
            Ok(Done::default())
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
            } else if op.get("pin").is_none() && to.is_empty() {
                f.out.clear();
            } else if !to.is_empty() {
                f.disconnect(&pin, &to);
            } else {
                f.out.retain(|p| p.name != pin.as_str());
            }
            Ok(Done::default())
        }
        "cut" => {
            let Some(list) = op.get("wires").filter(|w| w.is_table()) else {
                return Err("cut needs wires = { { from, pin, to }, ... }".into());
            };
            let mut n = 0;
            for w in list.arr() {
                let (from, pin, to) =
                    (w.get_string("from", ""), w.get_string("pin", "next"), w.get_string("to", ""));
                if let Some(f) = g.find_node_mut(&from)
                    && f.disconnect(&pin, &to)
                {
                    n += 1;
                }
            }
            Ok(Done::status(format!("Cut {n} wire{}", if n == 1 { "" } else { "s" })))
        }
        "set_pos" | "move" => {
            if let Some(by) = op.get("by").and_then(point_of) {
                let ids = op_nodes(op, sel);
                need_nodes(g, &ids)?;
                for id in &ids {
                    if let Some(n) = g.find_node_mut(id) {
                        n.pos = [layout::sane(snap(n.pos[0] + by[0])), layout::sane(snap(n.pos[1] + by[1]))];
                        n.has_pos = true;
                    }
                }
                return Ok(Done::default());
            }
            let id = op_node(op);
            let (x, y) = (op.get_number("x", 0.0) as f32, op.get_number("y", 0.0) as f32);
            let Some(n) = g.find_node_mut(&id) else { return Err(format!("node '{id}' does not exist")) };
            n.pos = [layout::sane(x), layout::sane(y)];
            n.has_pos = true;
            Ok(Done::default())
        }
        "replace" => {
            let id = op_node(op);
            let kind = op.get_string("kind", "");
            let Some(n) = g.find_node(&id).cloned() else {
                return Err(format!("node '{id}' does not exist"));
            };
            if kind.is_empty() {
                return Err("replace needs the new kind (kind=...)".into());
            }
            let trigger_now = cat.is_trigger(&kind);
            if trigger_now != cat.is_trigger(&n.kind) && g.in_degree(&id) > 0 && trigger_now {
                return Err(format!("'{id}' has wires into it, and a trigger takes none"));
            }
            let mut m = n.clone();
            m.kind = SmolStr::new(&kind);
            if let Some(k) = cat.find(&kind) {
                // the parameters of the same name and the wires of the same pin stay
                let mut params = Value::table();
                for (pk, pv) in n.params.fields() {
                    if k.param(pk).is_some() {
                        params.set(pk, pv.clone());
                    }
                }
                m.params = params;
                let (pins, declared) = layout::pins(cat, &m);
                let declared = pins.get(..declared).unwrap_or(&[]);
                m.out.retain(|p| declared.iter().any(|x| x == p.name.as_str()));
                if !k.is(crate::catalog::USE_MAIN) {
                    m.on_enter.clear();
                    m.on_exit.clear();
                }
            }
            if let Some(at) = g.node_index(&id) {
                g.nodes.set(at, m);
            }
            Ok(Done::select(vec![SmolStr::new(id)]))
        }
        "insert_on_wire" => {
            let (from, pin, to) =
                (op.get_string("from", ""), op.get_string("pin", "next"), op.get_string("to", ""));
            let linked = g
                .find_node(&from)
                .is_some_and(|f| f.targets(&pin).is_some_and(|t| t.iter().any(|x| x == &to)));
            if !linked {
                return Err(format!("there is no wire {from}.{pin} -> {to}"));
            }
            // a node already on the canvas goes on the wire where it is: nothing moves for it
            let existing = op.get_string("existing", "");
            if !existing.is_empty() {
                let Some(n) = g.find_node(&existing) else {
                    return Err(format!("node '{existing}' does not exist"));
                };
                if cat.is_trigger(&n.kind) {
                    return Err("a trigger takes no wire into it: it cannot go on a wire".into());
                }
                if existing == from || existing == to {
                    return Err(format!("'{existing}' is an end of that wire"));
                }
                let (npins, _) = layout::pins(cat, n);
                let Some(out_pin) = layout::default_pin(&npins).map(str::to_owned) else {
                    return Err(format!(
                        "{} has no pin to go on with",
                        text::kind_title(cat, &n.kind, g.is_behaviour())
                    ));
                };
                if let Some(m) = g.find_node_mut(&existing) {
                    m.connect(&out_pin, &to);
                }
                if let Some(f) = g.find_node_mut(&from) {
                    f.disconnect(&pin, &to);
                    f.connect(&pin, &existing);
                }
                g.canvas.set_route(&from, &pin, &to, Vec::new());
                return Ok(Done::select(vec![SmolStr::new(existing)]));
            }
            let mut n = new_node(cat, g, op)?;
            if g.find_node(&n.id).is_some() {
                return Err(format!("node '{}' already exists", n.id));
            }
            if cat.is_trigger(&n.kind) {
                return Err("a trigger takes no wire into it: it cannot go on a wire".into());
            }
            let cx = BoxCx::of(cat, g);
            let size = layout::node_size(&cx, &n);
            let place = match op.get("at").and_then(point_of) {
                Some(p) => p,
                None => {
                    let (a, b) = (g.find_node(&from), g.find_node(&to));
                    let ca = a.map_or([0.0, 0.0], |a| {
                        let s = layout::node_size(&cx, a);
                        [a.pos[0] + s[0] * 0.5, a.pos[1] + s[1] * 0.5]
                    });
                    let cb = b.map_or(ca, |b| {
                        let s = layout::node_size(&cx, b);
                        [b.pos[0] + s[0] * 0.5, b.pos[1] + s[1] * 0.5]
                    });
                    [(ca[0] + cb[0]) * 0.5 - size[0] * 0.5, (ca[1] + cb[1]) * 0.5 - size[1] * 0.5]
                }
            };
            n.pos = [snap(place[0]), snap(place[1])];
            n.has_pos = true;
            let (npins, _) = layout::pins(cat, &n);
            let Some(out_pin) = layout::default_pin(&npins).map(str::to_owned) else {
                return Err(format!(
                    "{} has no pin to go on with",
                    text::kind_title(cat, &n.kind, g.is_behaviour())
                ));
            };
            n.connect(&out_pin, &to);
            let nid = n.id.clone();
            if let Some(f) = g.find_node_mut(&from) {
                f.disconnect(&pin, &to);
                f.connect(&pin, &nid);
            }
            // what follows moves along the flow so the new node does not sit on it
            let depth = match cx.flow {
                Flow::Right => size[0],
                Flow::Down => size[1],
            } + arrange::gaps(cx.flow).depth;
            if let Some(ti) = g.node_index(&to) {
                let moving = arrange::branch(cat, g, ti);
                for i in moving {
                    if let Some(m) = g.nodes.get_mut(i) {
                        match cx.flow {
                            Flow::Right => m.pos[0] = snap(m.pos[0] + depth),
                            Flow::Down => m.pos[1] = snap(m.pos[1] + depth),
                        }
                    }
                }
            }
            g.nodes.push_back(n);
            Ok(Done::select(vec![nid]))
        }
        "add_action" | "set_action" | "remove_action" | "move_action" => {
            let id = op_node(op);
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
                return Ok(Done::default());
            }
            let Some(at) = slot_index(op, "index", size, false) else {
                return Err("index out of range (1-based)".into());
            };
            if what == "remove_action" {
                if let Some(n) = g.nodes.get_mut(ni) {
                    list_mut(n, exit).remove(at);
                }
                return Ok(Done::default());
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
                return Ok(Done::default());
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
            Ok(Done::default())
        }
        "set_quest" | "set_graph" => {
            let Some(patch) = op.get("patch").filter(|p| p.is_table()) else {
                return Err("set_graph needs patch = { id/title/activation/vars/tasks }".into());
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
                    other => return Err(format!("set_graph: unknown field '{other}'")),
                }
            }
            Ok(Done::default())
        }
        "set_task" => {
            let id = op.get_string("task", "");
            if !text::valid_id(&id) {
                return Err("set_task needs task = <id> ([a-z0-9_])".into());
            }
            let at = g.tasks.iter().position(|t| t.id == id);
            if op.get_bool("remove", false) {
                let Some(at) = at else { return Err(format!("task '{id}' does not exist")) };
                g.tasks.remove(at);
                return Ok(Done::default());
            }
            let patch = op.get("patch").cloned().unwrap_or_else(Value::table);
            let mut tv = match at.and_then(|i| g.tasks.get(i)) {
                Some(t) => task_value(t),
                None => Value::table(),
            };
            for (k, v) in patch.fields() {
                tv.set(k, v.clone());
            }
            let task = asset::task_from_value(&id, &tv);
            match at {
                Some(i) => {
                    g.tasks.set(i, task);
                }
                None => g.tasks.push_back(task),
            }
            Ok(Done::default())
        }
        "set_var" => {
            let name = op.get_string("name", "");
            if !text::valid_id(&name) {
                return Err("set_var needs name = <variable> ([a-z0-9_])".into());
            }
            let to = op.get_string("rename", "");
            let at = g.vars.iter().position(|v| v.name == name);
            if !to.is_empty() {
                let Some(i) = at else { return Err(format!("variable '{name}' does not exist")) };
                if g.find_var(&to).is_some() {
                    return Err(format!("variable '{to}' already exists"));
                }
                if let Some(v) = g.vars.get_mut(i) {
                    v.name = to;
                }
                return Ok(Done::default());
            }
            match op.get("value").filter(|v| !v.is_nil()) {
                None => {
                    if let Some(i) = at {
                        g.vars.remove(i);
                    }
                }
                Some(v) => match at.and_then(|i| g.vars.get_mut(i)) {
                    Some(var) => var.value = v.clone(),
                    None => g.vars.push(Var { name, value: v.clone() }),
                },
            }
            Ok(Done::default())
        }
        "frame" => frame_op(cat, g, op, sel),
        "route" => {
            let (from, pin, to) =
                (op.get_string("from", ""), op.get_string("pin", "next"), op.get_string("to", ""));
            let linked = g
                .find_node(&from)
                .is_some_and(|f| f.targets(&pin).is_some_and(|t| t.iter().any(|x| x == &to)));
            if !linked {
                return Err(format!("there is no wire {from}.{pin} -> {to}"));
            }
            let points: Vec<[f32; 2]> = op
                .get("points")
                .map(|p| {
                    p.arr()
                        .iter()
                        .filter_map(point_of)
                        .map(|p| [layout::sane(p[0]), layout::sane(p[1])])
                        .collect()
                })
                .unwrap_or_default();
            g.canvas.set_route(&from, &pin, &to, points);
            Ok(Done::default())
        }
        "bookmark" => {
            let mut node = op_node(op);
            // `here`: the selected node
            if node.is_empty() && op.get_bool("here", false) {
                node = sel.first().map(ToString::to_string).ok_or("select the node to bookmark")?;
            }
            let mut name = op.get_string("name", "");
            if node.is_empty() {
                if name.is_empty() {
                    return Err("bookmark needs a name (name=...) to remove, or a node to mark".into());
                }
                g.canvas.bookmarks.retain(|b| b.name != name);
                return Ok(Done::default());
            }
            if g.find_node(&node).is_none() {
                return Err(format!("node '{node}' does not exist"));
            }
            if name.is_empty() {
                name = node.clone();
            }
            match g.canvas.bookmarks.iter_mut().find(|b| b.name == name) {
                Some(b) => b.node = SmolStr::new(&node),
                None => g.canvas.bookmarks.push(Bookmark { name, node: SmolStr::new(&node) }),
            }
            Ok(Done::default())
        }
        "disable" | "collapse" => {
            let ids = op_nodes(op, sel);
            need_nodes(g, &ids)?;
            let first = ids
                .first()
                .and_then(|id| g.find_node(id))
                .is_some_and(|n| if what == "disable" { n.disabled } else { n.collapsed });
            let on = op.get("on").and_then(Value::as_bool).unwrap_or(!first);
            for id in &ids {
                if let Some(n) = g.find_node_mut(id) {
                    if what == "disable" {
                        n.disabled = on;
                    } else {
                        n.collapsed = on;
                    }
                }
            }
            Ok(Done::default())
        }
        "duplicate" => {
            let ids = op_nodes(op, sel);
            need_nodes(g, &ids)?;
            let src: Vec<Node> = ids.iter().filter_map(|id| g.find_node(id).cloned()).collect();
            let made = insert_nodes(cat, g, src, [GRID * 2.0, GRID * 2.0], op.get_bool("wires", false));
            Ok(Done::select(made))
        }
        "bypass" => {
            let ids = op_nodes(op, sel);
            let status = bypass(cat, g, &ids);
            Ok(Done::status(status))
        }
        "connect_nearest" => {
            let ids = op_nodes(op, sel);
            let (edge, status) = connect_nearest(cat, g, &ids);
            if let Some((from, pin, to)) = edge
                && let Some(f) = g.find_node_mut(&from)
            {
                f.connect(&pin, &to);
            }
            Ok(Done::status(status))
        }
        "wires" => {
            let w = &mut g.canvas.wires;
            let unset = |v: Option<&Value>| v.is_some_and(|v| v.is_nil() || v.as_str() == Some("default"));
            for key in ["style", "diagonal", "thickness", "by_pin", "dim", "bubbles", "arrows"] {
                let v = op.get(key);
                if v.is_none() {
                    continue;
                }
                let clear = unset(v);
                match key {
                    "style" => {
                        w.style = if clear {
                            None
                        } else {
                            Some(
                                v.and_then(Value::as_str)
                                    .and_then(WireStyle::parse)
                                    .ok_or("style must be curved, straight, circuit, angled or subway")?,
                            )
                        };
                    }
                    "diagonal" => {
                        w.diagonal = if clear {
                            None
                        } else {
                            Some(
                                v.and_then(Value::as_str)
                                    .and_then(Diagonal::parse)
                                    .ok_or("diagonal must be start, middle or end")?,
                            )
                        };
                    }
                    "thickness" => {
                        w.thickness =
                            if clear { None } else { v.map(|t| (t.to_number(2.0) as f32).clamp(1.0, 4.0)) };
                    }
                    _ => {
                        let b = if clear { None } else { v.and_then(Value::as_bool) };
                        match key {
                            "by_pin" => w.by_pin = b,
                            "dim" => w.dim = b,
                            "bubbles" => w.bubbles = b,
                            _ => w.arrows = b,
                        }
                    }
                }
            }
            Ok(Done::default())
        }
        "flow" => {
            let f = op.get_string("flow", "");
            g.canvas.flow = Some(Flow::parse(&f).ok_or("flow must be \"right\" or \"down\"")?);
            Ok(Done::default())
        }
        "paste" => {
            let text_ = op.get_string("text", "");
            let v = gz_luadata::parse(&text_).map_err(|e| format!("paste: {}", e.message))?;
            let Some(nodes) = v.get("nodes").filter(|n| n.is_table()) else {
                return Err("paste: the text holds no nodes table".into());
            };
            let mut shape = Vec::new();
            let src: Vec<Node> =
                nodes.arr().iter().filter_map(|nv| asset::node_from_value(cat, nv, &mut shape)).collect();
            if src.is_empty() {
                return Err("paste: no node to paste".into());
            }
            let skipped = refuse_foreign(cat, g, &src);
            let src: Vec<Node> = src.into_iter().filter(|n| !skipped.contains(&n.kind)).collect();
            let anchor = op.get("at").and_then(point_of).unwrap_or([0.0, 0.0]);
            let first = src.iter().find(|n| n.has_pos).map_or(anchor, |f| f.pos);
            let made = insert_nodes(cat, g, src, [anchor[0] - first[0], anchor[1] - first[1]], false);
            let mut done = Done::select(made);
            if !skipped.is_empty() {
                let names: Vec<String> =
                    skipped.iter().map(|k| text::kind_title(cat, k, g.is_behaviour())).collect();
                done.status = format!(
                    "Not pasted - a {} does not take: {}",
                    text::doc_noun(g.is_behaviour()),
                    names.join(", ")
                );
            }
            Ok(done)
        }
        other => Err(format!("unknown op '{other}'")),
    }
}

/// The kinds of a fragment the document may not carry (a quest's node pasted into a behaviour
/// graph): they are left out and named.
fn refuse_foreign(cat: &Catalog, g: &Graph, src: &[Node]) -> Vec<SmolStr> {
    let mut out: Vec<SmolStr> = Vec::new();
    for n in src {
        if cat.find(&n.kind).is_some_and(|k| !k.in_doc(g.is_behaviour())) && !out.contains(&n.kind) {
            out.push(n.kind.clone());
        }
    }
    out
}

fn task_value(t: &Task) -> Value {
    let mut g = Graph::default();
    g.tasks.push_back(t.clone());
    let v = asset::model_value(&Catalog::empty(), &g, &asset::AutoPos::default());
    v.get("tasks").and_then(|ts| ts.get(&t.id)).cloned().unwrap_or_else(Value::table)
}

/// `frame`: `action` add (around `nodes`, or at x/y/w/h), set (index + fields), remove (index),
/// move (index + by: the nodes inside move with it).
fn frame_op(cat: &Catalog, g: &mut Graph, op: &Value, sel: &[SmolStr]) -> Result<Done, String> {
    let action = op.get_string("action", "add");
    let index = || -> Result<usize, String> {
        let i = op.get_number("index", 0.0) as i64 - 1;
        usize::try_from(i)
            .ok()
            .filter(|&i| i < g.canvas.frames.len())
            .ok_or_else(|| "index out of range (1-based)".to_owned())
    };
    let fields = |f: &mut Frame| {
        if let Some(t) = op.get("title") {
            f.title = t.to_text("");
        }
        if let Some(t) = op.get("tone") {
            f.tone = t.to_number(0.0).clamp(0.0, 9.0) as u8;
        }
        for (key, slot) in [("x", &mut f.x), ("y", &mut f.y), ("w", &mut f.w), ("h", &mut f.h)] {
            if let Some(v) = op.get(key) {
                *slot = layout::sane(v.to_number(0.0) as f32);
            }
        }
        f.w = f.w.max(64.0);
        f.h = f.h.max(48.0);
    };
    match action.as_str() {
        "add" => {
            let mut f =
                Frame { title: "Comment".to_owned(), tone: 8, w: 320.0, h: 200.0, ..Frame::default() };
            let ids = op_nodes(op, sel);
            if !op.has("x") && !ids.is_empty() {
                need_nodes(g, &ids)?;
                let cx = BoxCx::of(cat, g);
                let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for id in &ids {
                    if let Some(n) = g.find_node(id) {
                        let s = layout::node_size(&cx, n);
                        r = [
                            r[0].min(n.pos[0]),
                            r[1].min(n.pos[1]),
                            r[2].max(n.pos[0] + s[0]),
                            r[3].max(n.pos[1] + s[1]),
                        ];
                    }
                }
                let pad = GRID * 2.0;
                // room for the title strip above the nodes
                f.x = snap(r[0] - pad);
                f.y = snap(r[1] - pad - GRID * 2.0);
                f.w = snap(r[2] - r[0] + pad * 2.0);
                f.h = snap(r[3] - r[1] + pad * 2.0 + GRID * 2.0);
            }
            fields(&mut f);
            g.canvas.frames.push(f);
            Ok(Done::default())
        }
        "set" => {
            let i = index()?;
            if let Some(f) = g.canvas.frames.get_mut(i) {
                fields(f);
            }
            Ok(Done::default())
        }
        "remove" => {
            let i = index()?;
            g.canvas.frames.remove(i);
            Ok(Done::default())
        }
        "move" => {
            let i = index()?;
            let by = op.get("by").and_then(point_of).ok_or("move needs by = { dx, dy }")?;
            let cx = BoxCx::of(cat, g);
            let Some(r) = g.canvas.frames.get(i).cloned() else { return Ok(Done::default()) };
            let inside: Vec<usize> = g
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    let s = layout::node_size(&cx, n);
                    r.holds([n.pos[0], n.pos[1], n.pos[0] + s[0], n.pos[1] + s[1]])
                })
                .map(|(k, _)| k)
                .collect();
            let ids: Vec<SmolStr> =
                inside.iter().filter_map(|&k| g.nodes.get(k).map(|n| n.id.clone())).collect();
            for k in inside {
                if let Some(n) = g.nodes.get_mut(k) {
                    n.pos = [layout::sane(n.pos[0] + by[0]), layout::sane(n.pos[1] + by[1])];
                }
            }
            // a wire with both ends inside goes along whole: its reroutes too
            let held = |id: &str| ids.iter().any(|x| x == id);
            for (key, points) in &mut g.canvas.routes {
                if crate::canvas::split_route_key(key).is_some_and(|(f, _, t)| held(f) && held(t)) {
                    for q in points.iter_mut() {
                        *q = [layout::sane(q[0] + by[0]), layout::sane(q[1] + by[1])];
                    }
                }
            }
            if let Some(f) = g.canvas.frames.get_mut(i) {
                f.x = layout::sane(f.x + by[0]);
                f.y = layout::sane(f.y + by[1]);
            }
            Ok(Done::default())
        }
        other => Err(format!("frame: action must be add, set, remove or move, not '{other}'")),
    }
}

/// A list of operations, all or nothing; a single operation without the outer list is accepted.
/// Returns how many were applied and what the last one left.
pub fn apply_ops(
    cat: &Catalog,
    g: &mut Graph,
    ops: &Value,
    sel: &[SmolStr],
) -> Result<(usize, Done), String> {
    if !ops.is_table() {
        return Err("ops must be a list of operations".into());
    }
    let mut work = g.clone();
    let mut done = Done::default();
    let mut selection = sel.to_vec();
    let applied = if ops.arr().is_empty() && ops.has("op") {
        done = apply_one(cat, &mut work, ops, &selection)?;
        1
    } else {
        for (i, op) in ops.arr().iter().enumerate() {
            let d = apply_one(cat, &mut work, op, &selection).map_err(|e| format!("op #{}: {e}", i + 1))?;
            if let Some(s) = &d.select {
                selection.clone_from(s);
            }
            if d.select.is_some() {
                done.select = d.select;
            }
            if !d.status.is_empty() {
                done.status = d.status;
            }
            done.renamed.extend(d.renamed);
        }
        ops.arr().len()
    };
    *g = work;
    Ok((applied, done))
}

/// Whether `from` may be wired to `to`: the target exists, is not the source and is no trigger.
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

/// The bridges and cuts of a bypass over `selection`: incoming edges in graph order paired with
/// outgoing ones in pin order; what has no counterpart goes.
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
/// declared pins only. Returns the edge and the status line.
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
    let cx = BoxCx::of(cat, g);
    let flow = cx.flow;
    let geo: Vec<Geo> = g
        .nodes
        .iter()
        .map(|n| {
            let (pins, declared) = layout::pins(cat, n);
            Geo { pos: n.pos, size: layout::node_size(&cx, n), pins, declared }
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
        let a = layout::output_pin(flow, sg.pos, sg.size, p, sg.pins.len());
        for (i, (n, ng)) in g.nodes.iter().zip(&geo).enumerate() {
            if i == si || !can_take_input(cat, g, n) {
                continue;
            }
            let d = dist2(a, layout::input_pin(flow, ng.pos, ng.size));
            if d < best {
                best = d;
                found = Some((sn.id.to_string(), pin.clone(), n.id.to_string()));
            }
        }
    }
    // somebody's free output into the selection
    if can_take_input(cat, g, sn) {
        let b = layout::input_pin(flow, sg.pos, sg.size);
        for (i, (o, og)) in g.nodes.iter().zip(&geo).enumerate() {
            if i == si {
                continue;
            }
            for p in 0..og.declared {
                let Some(pin) = og.pins.get(p) else { continue };
                if !pin_free(o, pin) {
                    continue;
                }
                let d = dist2(layout::output_pin(flow, og.pos, og.size, p, og.pins.len()), b);
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

/// The selection grown along the wires.
#[must_use]
pub fn grown(g: &Graph, selection: &[SmolStr], dir: Grow) -> Vec<SmolStr> {
    let index = g.index();
    let mut seen: Vec<bool> = vec![false; g.nodes.len()];
    let mut stack: Vec<usize> = selection.iter().filter_map(|s| index.get(s.as_str()).copied()).collect();
    for &i in &stack {
        seen[i] = true;
    }
    while let Some(i) = stack.pop() {
        let Some(n) = g.nodes.get(i) else { continue };
        if matches!(dir, Grow::Connected | Grow::Downstream) {
            for p in &n.out {
                for t in &p.targets {
                    if let Some(&j) = index.get(t.as_str())
                        && !seen[j]
                    {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if matches!(dir, Grow::Connected | Grow::Upstream) {
            for (j, m) in g.nodes.iter().enumerate() {
                if !seen[j] && m.has_target(&n.id) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    g.nodes.iter().zip(seen).filter(|(_, s)| *s).map(|(n, _)| n.id.clone()).collect()
}

/// The search results of a document: what the tab's Find shows.
#[must_use]
pub fn find_results(cat: &Catalog, g: &Graph, query: &str) -> Vec<FindHit> {
    let behaviour = g.is_behaviour();
    let title = |k: &str| text::kind_title(cat, k, behaviour);
    index::find_in_graph(g, &index::parse_query(query), &title)
}

/// The places an arrangement gives its nodes.
fn arrange_positions(
    cat: &Catalog,
    g: &Graph,
    a: &Arrange,
    sel: &[SmolStr],
) -> Result<Vec<(usize, [f32; 2])>, String> {
    let ids: Vec<SmolStr> =
        if a.nodes.is_empty() { sel.to_vec() } else { a.nodes.iter().map(SmolStr::new).collect() };
    let index = g.index();
    let set: Vec<usize> = ids.iter().filter_map(|id| index.get(id.as_str()).copied()).collect();
    if let Some(missing) = ids.iter().find(|id| !index.contains_key(id.as_str())) {
        return Err(format!("node '{missing}' does not exist"));
    }
    match a.action {
        ArrangeAction::Layout => match a.scope {
            Scope::All => {
                let all: Vec<usize> = (0..g.nodes.len()).collect();
                Ok(all.iter().copied().zip(arrange::layout(cat, g, &all)).collect())
            }
            Scope::Selection => {
                if set.is_empty() {
                    return Err("select the nodes to lay out".into());
                }
                Ok(arrange::layout_part(cat, g, &set, None))
            }
            Scope::Branch => {
                let [root] = set.as_slice() else {
                    return Err("select the one node whose branch to lay out".into());
                };
                let branch = arrange::branch(cat, g, *root);
                Ok(arrange::layout_part(cat, g, &branch, Some(*root)))
            }
            Scope::Tree => {
                if set.is_empty() {
                    return Err("select a node of the tree to lay out".into());
                }
                // each tree on its own, from its start: two selected nodes of one tree lay it out once
                let mut done = vec![false; g.nodes.len()];
                let mut out = Vec::new();
                for &i in &set {
                    if done.get(i).copied().unwrap_or(true) {
                        continue;
                    }
                    let (tree, root) = arrange::tree(cat, g, i);
                    for &t in &tree {
                        done[t] = true;
                    }
                    out.extend(arrange::layout_part(cat, g, &tree, Some(root)));
                }
                Ok(out)
            }
        },
        ArrangeAction::Align(how) => {
            if set.len() < 2 {
                return Err("select two nodes or more to align".into());
            }
            Ok(arrange::align(cat, g, &set, how))
        }
        ArrangeAction::Distribute { horizontal } => {
            if set.len() < 3 {
                return Err("select three nodes or more to distribute".into());
            }
            Ok(arrange::distribute(cat, g, &set, horizontal))
        }
        ArrangeAction::Straighten => {
            if set.is_empty() {
                return Err("select the nodes whose wires to straighten".into());
            }
            Ok(arrange::straighten(cat, g, &set))
        }
    }
}

impl GraphDoc {
    /// What follows every content change: pins in catalog order, places for nodes without one,
    /// routes of wires that went dropped, the selection pruned.
    fn changed(&mut self, cat: &Catalog) {
        // the load's shape problems are reported until the first edit
        self.shape = Arc::from(Vec::new());
        asset::canonicalize_all_pins(cat, &mut self.graph);
        layout::ensure_positions(cat, &mut self.graph);
        let g = self.graph.clone();
        self.graph.canvas.prune_routes(|f, p, t| {
            g.find_node(f).is_some_and(|n| n.targets(p).is_some_and(|x| x.iter().any(|y| y == t)))
        });
        self.graph.canvas.bookmarks.retain(|b| g.find_node(&b.node).is_some());
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

    fn after(&mut self, done: Done) {
        self.renames.extend(done.renamed);
        if let Some(s) = done.select {
            self.view.selection = s;
            self.view.slot.clear();
        }
        if !done.status.is_empty() {
            self.view.status = done.status;
        }
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
            ViewOp::Grow(dir) => {
                v.selection = grown(g, &v.selection, dir);
                v.slot.clear();
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
            ViewOp::BookmarkJump(delta) => {
                let marks: Vec<&Bookmark> =
                    g.canvas.bookmarks.iter().filter(|b| g.find_node(&b.node).is_some()).collect();
                if marks.is_empty() {
                    return Err(refused("the graph has no bookmarks"));
                }
                let len = marks.len() as i64;
                let current = match v.selection.as_slice() {
                    [one] => marks.iter().position(|b| b.node == *one).map_or(-1, |i| i as i64),
                    _ => -1,
                };
                let next =
                    if delta < 0 { if current < 0 { len - 1 } else { current - 1 } } else { current + 1 };
                let next = next.rem_euclid(len);
                let id = marks
                    .get(usize::try_from(next).unwrap_or(0))
                    .map(|b| b.node.to_string())
                    .unwrap_or_default();
                v.frame_node(cat, g, &id);
                reply.moved = true;
            }
            ViewOp::BookmarkGo(name) => {
                let marks = &g.canvas.bookmarks;
                let hit = marks.iter().find(|b| b.name == name).or_else(|| {
                    name.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| marks.get(i))
                });
                let Some(b) = hit else { return Err(refused(format!("no bookmark '{name}'"))) };
                let id = b.node.to_string();
                v.frame_node(cat, g, &id);
                reply.moved = true;
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

/// The undo label of a list of ops: the first op's.
fn ops_label(ops: &Value) -> &'static str {
    if ops.arr().is_empty() && ops.has("op") {
        return label_of(&ops.get_string("op", ""));
    }
    ops.arr().first().map_or("Edit Graph", |o| label_of(&o.get_string("op", "")))
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
                self.renames.clear();
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
                let label = ops_label(&ops);
                let (count, done) =
                    apply_ops(cat, &mut self.graph, &ops, &self.view.selection).map_err(refused)?;
                reply.count = count;
                if count == 0 {
                    cx.unchanged();
                    return Ok(reply);
                }
                self.after(done);
                self.changed(cat);
                cx.undoable(label);
            }
            DocOp::Apply(op) => {
                let label = label_of(&op.get_string("op", ""));
                let mut work = self.graph.clone();
                let done = apply_one(cat, &mut work, &op, &self.view.selection).map_err(refused)?;
                if work == self.graph {
                    self.after(done);
                    cx.unchanged();
                    return Ok(reply);
                }
                self.graph = work;
                self.after(done);
                self.changed(cat);
                cx.undoable(label);
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
                self.renames.push((SmolStr::new(&id), SmolStr::new(&new_id)));
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
            DocOp::Positions { at, label } => {
                let mut moved = 0;
                for (id, p) in &at {
                    if let Some(n) = self.graph.find_node_mut(id) {
                        let q = [layout::sane(p[0]), layout::sane(p[1])];
                        #[allow(clippy::float_cmp)]
                        if n.pos != q || !n.has_pos {
                            n.pos = q;
                            n.has_pos = true;
                            moved += 1;
                        }
                    }
                }
                reply.count = moved;
                if moved == 0 {
                    cx.unchanged();
                    return Ok(reply);
                }
                cx.undoable(label);
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
            DocOp::Arrange(a) => {
                let mut work = self.graph.clone();
                if let Some(f) = a.flow {
                    work.canvas.flow = Some(f);
                }
                let at = arrange_positions(cat, &work, &a, &self.view.selection).map_err(refused)?;
                for (i, p) in &at {
                    if let Some(n) = work.nodes.get_mut(*i) {
                        #[allow(clippy::float_cmp)]
                        if n.pos != *p || !n.has_pos {
                            reply.count += 1;
                        }
                        n.pos = [layout::sane(p[0]), layout::sane(p[1])];
                        n.has_pos = true;
                    }
                }
                if work == self.graph {
                    cx.unchanged();
                    return Ok(reply);
                }
                self.graph = work;
                self.changed(cat);
                cx.undoable(match (a.action, a.flow) {
                    (ArrangeAction::Layout, Some(_)) => "Flow Direction",
                    (ArrangeAction::Layout, None) => "Auto Layout",
                    (ArrangeAction::Align(_), _) => "Align",
                    (ArrangeAction::Distribute { .. }, _) => "Distribute",
                    (ArrangeAction::Straighten, _) => "Straighten Connections",
                });
            }
            DocOp::Canvas(op) => {
                let selection = self.view.selection.clone();
                match op {
                    CanvasOp::SelectAll => {
                        self.view.selection = self.graph.nodes.iter().map(|n| n.id.clone()).collect();
                    }
                    CanvasOp::Delete => {
                        if selection.is_empty()
                            || selection.iter().any(|id| self.graph.find_node(id).is_none())
                        {
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
                    CanvasOp::Duplicate { wires } => {
                        if selection.is_empty() {
                            cx.unchanged();
                            return Ok(reply);
                        }
                        let src: Vec<Node> =
                            selection.iter().filter_map(|id| self.graph.find_node(id).cloned()).collect();
                        let made = insert_nodes(cat, &mut self.graph, src, [GRID * 2.0, GRID * 2.0], wires);
                        self.view.selection = made;
                        self.view.slot.clear();
                        self.changed(cat);
                        cx.undoable(if wires { "Duplicate with Wires" } else { "Duplicate" });
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
                self.renames.clear();
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
mod tests;
