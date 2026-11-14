//! The graph as plain data: read from a declarative Lua value, projected back to one for a save
//! to compare ([`model_value`]: the save writes only the difference into the author's text),
//! written canonically for a new file ([`write`]), outlined for an answer.
//!
//! A `.nqasset` is a quest and a `.behasset` a behaviour graph; the header key says which (`nq`
//! or `beh`) and the extension has to agree (the validator's `E001`). Everything under `params`
//! stays a generic [`Value`] and is read through the [`Catalog`].

use gz_luadata::{Table, Value};
use imbl::Vector;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::canvas::{Canvas, Flow};
use crate::catalog::Catalog;
use crate::text::{self, preview};

/// Which of the two documents a graph is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DocKind {
    #[default]
    Quest,
    Behaviour,
}

impl DocKind {
    /// What the extension of `path` says: `.behasset` a behaviour graph, anything else a quest.
    #[must_use]
    pub fn of_path(path: &str) -> Self {
        if is_behaviour_path(path) { Self::Behaviour } else { Self::Quest }
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Quest => "quest",
            Self::Behaviour => "behaviour",
        }
    }
}

/// The extension of the last path segment, lower case, with its dot.
#[must_use]
pub fn extension(path: &str) -> String {
    let base = path.rsplit(['\\', '/']).next().unwrap_or(path);
    base.rfind('.').and_then(|d| base.get(d..)).map(str::to_ascii_lowercase).unwrap_or_default()
}

/// The file name without folders and without its last extension.
#[must_use]
pub fn base_name(path: &str) -> String {
    let base = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match base.rfind('.') {
        Some(d) if d > 0 => base.get(..d).unwrap_or(base).to_owned(),
        _ => base.to_owned(),
    }
}

#[must_use]
pub fn is_quest_path(path: &str) -> bool {
    extension(path) == ".nqasset"
}

#[must_use]
pub fn is_behaviour_path(path: &str) -> bool {
    extension(path) == ".behasset"
}

#[must_use]
pub fn is_graph_path(path: &str) -> bool {
    is_quest_path(path) || is_behaviour_path(path)
}

/// An extra action: `{ kind = ..., params = { ... } }`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Action {
    pub kind: SmolStr,
    pub params: Value,
}

/// A condition; `["not"] = true` inverts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cond {
    pub kind: SmolStr,
    pub params: Value,
    pub negate: bool,
}

/// An output pin and its targets, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pin {
    pub name: SmolStr,
    pub targets: Vec<SmolStr>,
}

/// A node of the graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub id: SmolStr,
    pub kind: SmolStr,
    pub once: bool,
    /// `once` is written in the file; otherwise the catalog default applies.
    pub once_set: bool,
    pub params: Value,
    pub cond: Vec<Cond>,
    pub on_enter: Vec<Action>,
    pub on_exit: Vec<Action>,
    pub out: Vec<Pin>,
    pub pos: [f32; 2],
    pub has_pos: bool,
    pub comment: String,
    /// Drawn as its header and pins alone (`collapsed = true`, the editor's).
    pub collapsed: bool,
    /// The game passes it through without running it (`disabled = true`).
    pub disabled: bool,
}

impl Node {
    #[must_use]
    pub fn new(id: &str, kind: &str) -> Self {
        Self { id: SmolStr::new(id), kind: SmolStr::new(kind), ..Self::default() }
    }

    #[must_use]
    pub fn targets(&self, pin: &str) -> Option<&[SmolStr]> {
        self.out.iter().find(|p| p.name == pin).map(|p| p.targets.as_slice())
    }

    /// Adds a link; false when it is there already.
    pub fn connect(&mut self, pin: &str, to: &str) -> bool {
        let at = match self.out.iter().position(|p| p.name == pin) {
            Some(at) => at,
            None => {
                self.out.push(Pin { name: SmolStr::new(pin), targets: Vec::new() });
                self.out.len() - 1
            }
        };
        let Some(p) = self.out.get_mut(at) else { return false };
        if p.targets.iter().any(|t| t == to) {
            return false;
        }
        p.targets.push(SmolStr::new(to));
        true
    }

    /// Drops a link; a pin left without targets goes too. False when it was not there.
    pub fn disconnect(&mut self, pin: &str, to: &str) -> bool {
        let Some(at) = self.out.iter().position(|p| p.name == pin) else { return false };
        let Some(p) = self.out.get_mut(at) else { return false };
        let Some(t) = p.targets.iter().position(|t| t == to) else { return false };
        p.targets.remove(t);
        if p.targets.is_empty() {
            self.out.remove(at);
        }
        true
    }

    #[must_use]
    pub fn has_target(&self, to: &str) -> bool {
        self.out.iter().any(|p| p.targets.iter().any(|t| t == to))
    }

    #[must_use]
    pub fn out_degree(&self) -> usize {
        self.out.iter().map(|p| p.targets.len()).sum()
    }

    /// `enter` / `on_enter` or `exit` / `on_exit`, any case.
    #[must_use]
    pub fn slot(&self, slot: &str) -> Option<&Vec<Action>> {
        if slot.eq_ignore_ascii_case("enter") || slot.eq_ignore_ascii_case("on_enter") {
            Some(&self.on_enter)
        } else if slot.eq_ignore_ascii_case("exit") || slot.eq_ignore_ascii_case("on_exit") {
            Some(&self.on_exit)
        } else {
            None
        }
    }
}

/// One step of a task: its own text and its own map spot.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub id: String,
    pub title: Value,
    pub descr: Value,
    pub target: Value,
    pub visible: bool,
}

impl Default for Objective {
    fn default() -> Self {
        Self { id: String::new(), title: Value::Nil, descr: Value::Nil, target: Value::Nil, visible: true }
    }
}

/// A PDA task the quest declares.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: Value,
    pub descr: Value,
    /// `additional` | `storyline`.
    pub ty: String,
    pub target: Value,
    pub icon: String,
    pub objectives: Vec<Objective>,
    /// Every step completed completes the task itself.
    pub auto_complete: bool,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: Value::Nil,
            descr: Value::Nil,
            ty: String::new(),
            target: Value::Nil,
            icon: String::new(),
            objectives: Vec::new(),
            auto_complete: true,
        }
    }
}

impl Task {
    #[must_use]
    pub fn objective(&self, id: &str) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.id == id)
    }
}

/// A variable of the graph with its default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Var {
    pub name: String,
    pub value: Value,
}

/// A role of a behaviour graph: a slot its caller fills.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Role {
    pub name: String,
    pub title: Value,
    pub many: bool,
    pub optional: bool,
    /// The binding when the caller names none.
    pub default: Value,
}

/// A quest or a behaviour graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// The format version of the header key; 0 when the key is missing.
    pub nq: i32,
    pub id: String,
    pub kind: DocKind,
    pub title: Value,
    /// `auto` | `manual`.
    pub activation: String,
    pub vars: Vec<Var>,
    pub roles: Vec<Role>,
    pub tasks: Vector<Task>,
    pub nodes: Vector<Node>,
    /// The editor's own table: flow, wires, frames, routes, bookmarks.
    pub canvas: Canvas,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            nq: 1,
            id: String::new(),
            kind: DocKind::Quest,
            title: Value::Nil,
            activation: "auto".to_owned(),
            vars: Vec::new(),
            roles: Vec::new(),
            tasks: Vector::new(),
            nodes: Vector::new(),
            canvas: Canvas::default(),
        }
    }
}

/// Node ids to their index; the first of two equal ids wins.
pub type NodeIndex<'a> = FxHashMap<&'a str, usize>;

impl Graph {
    #[must_use]
    pub fn is_behaviour(&self) -> bool {
        self.kind == DocKind::Behaviour
    }

    /// The way the graph flows.
    #[must_use]
    pub fn flow(&self) -> Flow {
        self.canvas.flow()
    }

    #[must_use]
    pub fn index(&self) -> NodeIndex<'_> {
        let mut m = FxHashMap::default();
        m.reserve(self.nodes.len());
        for (i, n) in self.nodes.iter().enumerate() {
            m.entry(n.id.as_str()).or_insert(i);
        }
        m
    }

    #[must_use]
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    #[must_use]
    pub fn find_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn find_node_mut(&mut self, id: &str) -> Option<&mut Node> {
        let at = self.node_index(id)?;
        self.nodes.get_mut(at)
    }

    #[must_use]
    pub fn find_task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    #[must_use]
    pub fn find_var(&self, name: &str) -> Option<&Var> {
        self.vars.iter().find(|v| v.name == name)
    }

    /// Edges arriving at `id`, over every pin of every node.
    #[must_use]
    pub fn in_degree(&self, id: &str) -> usize {
        self.nodes
            .iter()
            .map(|n| n.out.iter().map(|p| p.targets.iter().filter(|t| *t == id).count()).sum::<usize>())
            .sum()
    }

    /// The nodes reachable from the triggers, or from `from`, by index.
    #[must_use]
    pub fn reachable(&self, catalog: &Catalog, from: Option<&str>) -> Vec<bool> {
        let idx = self.index();
        let mut out = vec![false; self.nodes.len()];
        let mut stack: Vec<usize> = Vec::new();
        match from {
            Some(f) => {
                if let Some(&i) = idx.get(f) {
                    out[i] = true;
                    stack.push(i);
                }
            }
            None => {
                for (i, n) in self.nodes.iter().enumerate() {
                    if catalog.is_trigger(&n.kind) {
                        out[i] = true;
                        stack.push(i);
                    }
                }
            }
        }
        while let Some(i) = stack.pop() {
            let Some(n) = self.nodes.get(i) else { continue };
            for p in &n.out {
                for t in &p.targets {
                    if let Some(&j) = idx.get(t.as_str())
                        && let Some(seen) = out.get_mut(j)
                        && !*seen
                    {
                        *seen = true;
                        stack.push(j);
                    }
                }
            }
        }
        out
    }

    /// The phrases reachable from a topic through `next` pins of phrases, with their depth (0 =
    /// the topic); a phrase met at both parities is reported at both.
    #[must_use]
    pub fn dialog_chain(&self, topic: &str) -> Vec<(usize, u32)> {
        let idx = self.index();
        let Some(&root) = idx.get(topic) else { return Vec::new() };
        let mut seen = vec![0u8; self.nodes.len()];
        let mut queue: Vec<(usize, u32)> = vec![(root, 0)];
        if let Some(s) = seen.get_mut(root) {
            *s |= 1;
        }
        let mut q = 0;
        while let Some(&(node, depth)) = queue.get(q) {
            q += 1;
            let Some(n) = self.nodes.get(node) else { continue };
            for p in &n.out {
                // only `next` carries phrases
                if p.name != "next" {
                    continue;
                }
                for t in &p.targets {
                    let Some(&j) = idx.get(t.as_str()) else { continue };
                    if !self.nodes.get(j).is_some_and(|m| text::is_phrase_kind(&m.kind)) {
                        continue;
                    }
                    let d = depth + 1;
                    let bit = if d & 1 == 1 { 2 } else { 1 };
                    let Some(s) = seen.get_mut(j) else { continue };
                    if *s & bit != 0 {
                        continue;
                    }
                    *s |= bit;
                    queue.push((j, d));
                }
            }
        }
        queue
    }

    /// The stable topological order from the triggers: siblings stay together, a
    /// dialog is written as one block before the world nodes it leads to, unreachable nodes keep
    /// their order at the end.
    #[must_use]
    pub fn topo_order(&self, catalog: &Catalog) -> Vec<usize> {
        let idx = self.index();
        let count = self.nodes.len();
        let mut out = Vec::with_capacity(count);
        let mut done = vec![false; count];
        let mut stack: Vec<usize> = Vec::new();
        let mut deferred: Vec<usize> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if !catalog.is_trigger(&n.kind) || done[i] {
                continue;
            }
            done[i] = true;
            out.push(i);
            stack.clear();
            deferred.clear();
            stack.push(i);
            loop {
                if stack.is_empty() {
                    if deferred.is_empty() {
                        break;
                    }
                    out.extend_from_slice(&deferred);
                    stack.extend(deferred.iter().rev());
                    deferred.clear();
                }
                let Some(cur) = stack.pop() else { break };
                let Some(n) = self.nodes.get(cur) else { continue };
                let from_dialog = text::is_dialog_kind(&n.kind);
                let mut kids: Vec<usize> = Vec::new();
                for p in &n.out {
                    for t in &p.targets {
                        let Some(&j) = idx.get(t.as_str()) else { continue };
                        let Some(d) = done.get_mut(j) else { continue };
                        if *d {
                            continue;
                        }
                        *d = true;
                        let phrase = self.nodes.get(j).is_some_and(|m| text::is_phrase_kind(&m.kind));
                        if from_dialog && !phrase {
                            deferred.push(j);
                        } else {
                            kids.push(j);
                        }
                    }
                }
                out.extend_from_slice(&kids);
                stack.extend(kids.iter().rev());
            }
        }
        for (i, d) in done.iter().enumerate() {
            if !d {
                out.push(i);
            }
        }
        out
    }

    /// A node id derived from `base` that is free: `step`, `step2`, ....
    #[must_use]
    pub fn free_node_id(&self, base: &str) -> String {
        let b = if base.is_empty() { "node" } else { base };
        if self.find_node(b).is_none() {
            return b.to_owned();
        }
        for i in 2..10000 {
            let cand = format!("{b}{i}");
            if self.find_node(&cand).is_none() {
                return cand;
            }
        }
        b.to_owned()
    }
}

// ---- reading ----------------------------------------------------------------------------------

/// An action from its table.
pub fn action_from_value(v: &Value) -> Result<Action, String> {
    if !v.is_table() {
        return Err("action must be a table { kind = ..., params = { ... } }".to_owned());
    }
    let kind = match v.get("kind") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return Err("action without kind".to_owned()),
    };
    let mut a = Action { kind, params: Value::Nil };
    if let Some(p) = v.get("params") {
        if !p.is_table() && !p.is_nil() {
            return Err("action params must be a table".to_owned());
        }
        a.params = p.clone();
    }
    Ok(a)
}

/// A condition from its table.
pub fn cond_from_value(v: &Value) -> Result<Cond, String> {
    if !v.is_table() {
        return Err("condition must be a table { kind = ..., params = { ... } }".to_owned());
    }
    let kind = match v.get("kind") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return Err("condition without kind".to_owned()),
    };
    let mut c = Cond { kind, params: Value::Nil, negate: false };
    if let Some(p) = v.get("params") {
        if !p.is_table() && !p.is_nil() {
            return Err("condition params must be a table".to_owned());
        }
        c.params = p.clone();
    }
    if let Some(n) = v.get("not") {
        c.negate = n.to_bool(false);
    }
    Ok(c)
}

/// A task from its table.
#[must_use]
pub fn task_from_value(id: &str, v: &Value) -> Task {
    let mut t = Task { id: id.to_owned(), ..Task::default() };
    if !v.is_table() {
        return t;
    }
    if let Some(x) = v.get("title") {
        t.title = x.clone();
    }
    if let Some(x) = v.get("descr") {
        t.descr = x.clone();
    }
    t.ty = v.get_string("type", "additional");
    if let Some(x) = v.get("target") {
        t.target = x.clone();
    }
    t.icon = v.get_string("icon", "");
    if let Some(x) = v.get("auto_complete") {
        t.auto_complete = x.to_bool(true);
    }
    if let Some(objs) = v.get("objectives") {
        for o in objs.arr() {
            let mut obj = Objective { id: o.get_string("id", ""), ..Objective::default() };
            if let Some(x) = o.get("title") {
                obj.title = x.clone();
            }
            if let Some(x) = o.get("descr") {
                obj.descr = x.clone();
            }
            if let Some(x) = o.get("target") {
                obj.target = x.clone();
            }
            if let Some(x) = o.get("visible") {
                obj.visible = x.to_bool(true);
            }
            t.objectives.push(obj);
        }
    }
    t
}

fn pin_rank(catalog: &Catalog, n: &Node, pin: &str) -> usize {
    if let Some(k) = catalog.find(&n.kind) {
        for (i, p) in k.pins.iter().enumerate() {
            if p == pin {
                return i * 1000;
            }
            if p != "cases" {
                continue;
            }
            let Some(cases) = n.params.get("cases").filter(|c| c.is_table()) else { continue };
            for (c, case) in cases.arr().iter().enumerate() {
                if case.get_string("name", "") == pin {
                    return i * 1000 + c + 1;
                }
            }
        }
    }
    1_000_000
}

/// Orders a node's pins by the catalog: case pins in case order, then the declared pins, the
/// unknown ones last by name.
pub fn canonicalize_pins(catalog: &Catalog, n: &mut Node) {
    if n.out.len() < 2 {
        return;
    }
    let ranks: Vec<usize> = n.out.iter().map(|p| pin_rank(catalog, n, &p.name)).collect();
    let mut order: Vec<usize> = (0..n.out.len()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (ranks.get(a).copied().unwrap_or(0), ranks.get(b).copied().unwrap_or(0));
        let (na, nb) = (n.out.get(a).map(|p| p.name.as_bytes()), n.out.get(b).map(|p| p.name.as_bytes()));
        ra.cmp(&rb).then_with(|| na.cmp(&nb))
    });
    let pins = std::mem::take(&mut n.out);
    let mut slots: Vec<Option<Pin>> = pins.into_iter().map(Some).collect();
    n.out = order.into_iter().filter_map(|i| slots.get_mut(i).and_then(Option::take)).collect();
}

/// Every node's pins in catalog order.
pub fn canonicalize_all_pins(catalog: &Catalog, g: &mut Graph) {
    for n in g.nodes.iter_mut() {
        if n.out.len() >= 2 {
            canonicalize_pins(catalog, n);
        }
    }
}

/// A node from its table: `None` when it is no node at all; what it holds but
/// cannot be typed is reported in `errs` and left out.
pub fn node_from_value(catalog: &Catalog, v: &Value, errs: &mut Vec<String>) -> Option<Node> {
    if !v.is_table() {
        errs.push("node must be a table".to_owned());
        return None;
    }
    let id = match v.get("id") {
        Some(Value::String(s)) => s.clone(),
        _ => {
            errs.push("node without id".to_owned());
            return None;
        }
    };
    let mut n = Node { id: id.clone(), kind: SmolStr::new(v.get_string("kind", "")), ..Node::default() };
    if let Some(o) = v.get("once") {
        match o {
            Value::Bool(b) => {
                n.once = *b;
                n.once_set = true;
            }
            _ => errs.push(format!("node '{id}': once must be true or false")),
        }
    }
    if let Some(p) = v.get("params") {
        if p.is_table() {
            n.params = p.clone();
        } else if !p.is_nil() {
            errs.push(format!("node '{id}': params must be a table"));
        }
    }
    if let Some(c) = v.get("cond") {
        if !c.is_table() {
            errs.push(format!("node '{id}': cond must be a list of conditions"));
        } else {
            // a single condition written without the outer list is accepted
            if c.arr().is_empty() && c.has("kind") {
                match cond_from_value(c) {
                    Ok(cc) => n.cond.push(cc),
                    Err(e) => errs.push(format!("node '{id}': cond: {e}")),
                }
            }
            for (i, e) in c.arr().iter().enumerate() {
                match cond_from_value(e) {
                    Ok(cc) => n.cond.push(cc),
                    Err(err) => errs.push(format!("node '{id}': cond #{}: {err}", i + 1)),
                }
            }
        }
    }
    for (slot, exit) in [("on_enter", false), ("on_exit", true)] {
        let Some(lst) = v.get(slot) else { continue };
        if !lst.is_table() {
            errs.push(format!("node '{id}': {slot} must be a list of actions"));
            continue;
        }
        let mut got: Vec<Action> = Vec::new();
        if lst.arr().is_empty() && lst.has("kind") {
            match action_from_value(lst) {
                Ok(a) => got.push(a),
                Err(e) => errs.push(format!("node '{id}': {slot}: {e}")),
            }
        }
        for (i, e) in lst.arr().iter().enumerate() {
            match action_from_value(e) {
                Ok(a) => got.push(a),
                Err(err) => errs.push(format!("node '{id}': {slot} #{}: {err}", i + 1)),
            }
        }
        if exit {
            n.on_exit.extend(got);
        } else {
            n.on_enter.extend(got);
        }
    }
    if let Some(out) = v.get("out") {
        if !out.is_table() {
            errs.push(format!("node '{id}': out must be a table {{ pin = target | {{ targets }} }}"));
        } else {
            if !out.arr().is_empty() {
                errs.push(format!("node '{id}': out must use named pins"));
            }
            for (pin, t) in out.fields() {
                let mut targets: Vec<SmolStr> = Vec::new();
                match t {
                    Value::String(s) => targets.push(s.clone()),
                    Value::Table(tt) => {
                        for (k, x) in tt.arr.iter().enumerate() {
                            match x {
                                Value::String(s) => targets.push(s.clone()),
                                _ => errs.push(format!(
                                    "node '{id}': out.{pin}: target #{} is not a node id",
                                    k + 1
                                )),
                            }
                        }
                    }
                    Value::Nil => {}
                    _ => errs.push(format!("node '{id}': out.{pin} must be a node id or a list of ids")),
                }
                if !targets.is_empty() {
                    n.out.push(Pin { name: pin.clone(), targets });
                }
            }
        }
    }
    if let Some(pos) = v.get("pos") {
        if pos.is_table() && pos.arr().len() >= 2 {
            let a = pos.arr();
            n.pos = [
                a.first().map_or(0.0, |x| x.to_number(0.0)) as f32,
                a.get(1).map_or(0.0, |x| x.to_number(0.0)) as f32,
            ];
            n.has_pos = true;
        } else if pos.is_table() && pos.has("x") && pos.has("y") {
            n.pos = [pos.get_number("x", 0.0) as f32, pos.get_number("y", 0.0) as f32];
            n.has_pos = true;
        } else if !pos.is_nil() {
            errs.push(format!("node '{id}': pos must be {{ x, y }}"));
        }
    }
    n.comment = v.get_string("comment", "");
    for (key, slot) in [("collapsed", &mut n.collapsed), ("disabled", &mut n.disabled)] {
        match v.get(key) {
            None | Some(Value::Nil) => {}
            Some(Value::Bool(b)) => *slot = *b,
            Some(_) => errs.push(format!("node '{id}': {key} must be true or false")),
        }
    }
    canonicalize_pins(catalog, &mut n);
    Some(n)
}

/// The typed graph of a file's value. Shape problems are collected as
/// messages; the rest is read anyway, so the author sees as much as can be read.
#[must_use]
pub fn graph_from_value(catalog: &Catalog, v: &Value) -> (Graph, Vec<String>) {
    let mut errs = Vec::new();
    let mut q = Graph::default();
    if !v.is_table() {
        errs.push("the file must return one table".to_owned());
        return (q, errs);
    }
    let ver_nq = v.get("nq");
    let ver_beh = v.get("beh");
    if ver_nq.is_some() && ver_beh.is_some() {
        errs.push("a document is a quest (nq) or a behaviour graph (beh), not both".to_owned());
    }
    q.kind = if ver_beh.is_some() && ver_nq.is_none() { DocKind::Behaviour } else { DocKind::Quest };
    let ver = if q.is_behaviour() { ver_beh } else { ver_nq };
    q.nq = match ver {
        Some(Value::Number(n)) => *n as i32,
        Some(x) => {
            let n = x.to_number(-1.0) as i32;
            if n < 0 {
                errs.push("the version must be a number".to_owned());
            }
            n
        }
        // absent: E001 by the validator
        None => 0,
    };
    if v.has("type") {
        errs.push(
            "there is no type key: .nqasset + nq is a quest, .behasset + beh is a behaviour graph".to_owned(),
        );
    }
    q.id = v.get_string("id", "");
    if let Some(x) = v.get("title") {
        q.title = x.clone();
    }
    q.activation = v.get_string("activation", "auto");
    if let Some(roles) = v.get("roles") {
        if !roles.is_table() || !roles.arr().is_empty() {
            errs.push("roles must be a table { name = { title, many, optional, default } }".to_owned());
        } else {
            for (name, rv) in roles.fields() {
                if !rv.is_table() {
                    errs.push(format!("role '{name}' must be a table"));
                    continue;
                }
                let mut r = Role { name: name.to_string(), ..Role::default() };
                if let Some(t) = rv.get("title") {
                    r.title = t.clone();
                }
                r.many = rv.get_bool("many", false);
                r.optional = rv.get_bool("optional", false);
                if let Some(d) = rv.get("default") {
                    r.default = d.clone();
                }
                q.roles.push(r);
            }
        }
    }
    if let Some(vars) = v.get("vars") {
        if !vars.is_table() {
            errs.push("vars must be a table { name = default }".to_owned());
        } else {
            if !vars.arr().is_empty() {
                errs.push("vars must use named keys".to_owned());
            }
            for (name, value) in vars.fields() {
                q.vars.push(Var { name: name.to_string(), value: value.clone() });
            }
        }
    }
    if let Some(tasks) = v.get("tasks") {
        if !tasks.is_table() {
            errs.push("tasks must be a table { id = { title = ..., ... } }".to_owned());
        } else {
            if !tasks.arr().is_empty() {
                errs.push("tasks must use named keys".to_owned());
            }
            for (id, tv) in tasks.fields() {
                if !tv.is_table() {
                    errs.push(format!("task '{id}' must be a table"));
                }
                q.tasks.push_back(task_from_value(id, tv));
            }
        }
    }
    match v.get("nodes") {
        Some(nodes) if !nodes.is_table() => errs.push("nodes must be a list of node tables".to_owned()),
        Some(nodes) => {
            for (i, nv) in nodes.arr().iter().enumerate() {
                let before = errs.len();
                match node_from_value(catalog, nv, &mut errs) {
                    Some(n) => q.nodes.push_back(n),
                    None => {
                        if errs.len() > before
                            && let Some(last) = errs.last_mut()
                        {
                            last.push_str(&format!(" (nodes #{})", i + 1));
                        }
                    }
                }
            }
            if !nodes.fields().is_empty() {
                errs.push("nodes must be a list, not a keyed table".to_owned());
            }
        }
        None => errs.push("nodes list is missing".to_owned()),
    }
    q.canvas = Canvas::from_value(v.get("canvas"), &mut errs);
    // a file with positions was laid out top to bottom; one without takes the preference later
    q.canvas.implied = if q.nodes.iter().any(|n| n.has_pos) { Flow::Down } else { Flow::Right };
    (q, errs)
}

/// Why a text is no graph: the message and its position (0 = unknown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl ReadError {
    /// The message with ` (line L)` after it.
    #[must_use]
    pub fn with_line(&self) -> String {
        if self.line > 0 { format!("{} (line {})", self.message, self.line) } else { self.message.clone() }
    }

    /// The message with ` (line L:C)` or ` (line L)` after it.
    #[must_use]
    pub fn with_position(&self) -> String {
        if self.line > 0 && self.column > 0 {
            format!("{} (line {}:{})", self.message, self.line, self.column)
        } else {
            self.with_line()
        }
    }
}

/// The value of a file, or why it is no declarative table. `chunk` is the file's
/// base name.
pub fn parse_value(text: &str, chunk: &str) -> Result<Value, ReadError> {
    gz_luadata::parse_named(text, chunk).map_err(|e| ReadError {
        message: e.message,
        line: e.line,
        column: e.column,
    })
}

/// A graph from its text; any shape problem fails it, one message per line.
pub fn parse(catalog: &Catalog, text: &str, chunk: &str) -> Result<Graph, ReadError> {
    let v = parse_value(text, chunk)?;
    let (g, shape) = graph_from_value(catalog, &v);
    if !shape.is_empty() {
        return Err(ReadError { message: shape.join("\n"), line: 0, column: 0 });
    }
    Ok(g)
}

// ---- writing ----------------------------------------------------------------------------------

fn put(t: &mut Table, key: &str, v: impl Into<Value>) {
    t.put(key, v);
}

/// An action as the writer emits it.
#[must_use]
pub fn action_to_value(a: &Action) -> Value {
    let mut t = Table::new();
    put(&mut t, "kind", a.kind.clone());
    if !a.params.is_empty() {
        put(&mut t, "params", a.params.clone());
    }
    Value::Table(t)
}

/// A condition as the writer emits it.
#[must_use]
pub fn cond_to_value(c: &Cond) -> Value {
    let mut t = Table::new();
    put(&mut t, "kind", c.kind.clone());
    if !c.params.is_empty() {
        put(&mut t, "params", c.params.clone());
    }
    if c.negate {
        put(&mut t, "not", true);
    }
    Value::Table(t)
}

/// A node as the writer emits it.
#[must_use]
pub fn node_to_value(n: &Node) -> Value {
    let mut t = Table::new();
    put(&mut t, "id", n.id.clone());
    put(&mut t, "kind", n.kind.clone());
    if n.once_set {
        put(&mut t, "once", n.once);
    }
    if n.disabled {
        put(&mut t, "disabled", true);
    }
    if !n.params.is_empty() {
        put(&mut t, "params", n.params.clone());
    }
    if !n.cond.is_empty() {
        put(&mut t, "cond", Value::from(n.cond.iter().map(cond_to_value).collect::<Vec<_>>()));
    }
    if !n.on_enter.is_empty() {
        put(&mut t, "on_enter", Value::from(n.on_enter.iter().map(action_to_value).collect::<Vec<_>>()));
    }
    if !n.on_exit.is_empty() {
        put(&mut t, "on_exit", Value::from(n.on_exit.iter().map(action_to_value).collect::<Vec<_>>()));
    }
    if n.out.iter().any(|p| !p.targets.is_empty()) {
        let mut o = Table::new();
        for p in &n.out {
            match p.targets.as_slice() {
                [] => {}
                [one] => put(&mut o, &p.name, one.clone()),
                many => put(
                    &mut o,
                    &p.name,
                    Value::from(many.iter().map(|t| Value::from(t.clone())).collect::<Vec<_>>()),
                ),
            }
        }
        put(&mut t, "out", o);
    }
    if n.has_pos {
        put(&mut t, "pos", Value::from(vec![Value::from(n.pos[0]), Value::from(n.pos[1])]));
    }
    if n.collapsed {
        put(&mut t, "collapsed", true);
    }
    if !n.comment.is_empty() {
        put(&mut t, "comment", n.comment.as_str());
    }
    Value::Table(t)
}

/// One node as a Lua table literal.
#[must_use]
pub fn write_node(n: &Node, indent: usize) -> String {
    gz_luadata::write_node(&node_to_value(n), indent)
}

/// The whole graph as the tables a new file is written from: the nodes in their topological
/// order.
#[must_use]
pub fn graph_to_value(catalog: &Catalog, q: &Graph) -> Table {
    graph_table(catalog, q, true, &AutoPos::default())
}

/// Positions the layout gave nodes the file wrote none for: what a save leaves out, so a file
/// without coordinates stays without them until the author moves a node.
pub type AutoPos = FxHashMap<SmolStr, [f32; 2]>;

/// The graph as a save compares it with what was read: the nodes in the model's order, the
/// positions the layout chose left out.
#[must_use]
pub fn model_value(catalog: &Catalog, q: &Graph, auto: &AutoPos) -> Value {
    Value::Table(graph_table(catalog, q, false, auto))
}

/// The nodes of `q` that have no position of their own, by id.
#[must_use]
pub fn unplaced(q: &Graph) -> Vec<SmolStr> {
    q.nodes.iter().filter(|n| !n.has_pos).map(|n| n.id.clone()).collect()
}

fn graph_table(catalog: &Catalog, q: &Graph, topo: bool, auto: &AutoPos) -> Table {
    let mut out = Table::new();
    // the header key is the kind: nothing else in the file says it
    put(&mut out, if q.is_behaviour() { "beh" } else { "nq" }, q.nq);
    put(&mut out, "id", q.id.as_str());
    // an absent title stays absent: an empty string is a title, and the journal would show nothing
    if !q.title.is_nil() {
        put(&mut out, "title", q.title.clone());
    }
    if !q.is_behaviour() {
        put(&mut out, "activation", q.activation.as_str());
    }
    if !q.roles.is_empty() {
        let mut roles = Table::new();
        for r in &q.roles {
            let mut rv = Table::new();
            if !r.title.is_nil() {
                put(&mut rv, "title", r.title.clone());
            }
            if r.many {
                put(&mut rv, "many", true);
            }
            if r.optional {
                put(&mut rv, "optional", true);
            }
            if !r.default.is_nil() {
                put(&mut rv, "default", r.default.clone());
            }
            put(&mut roles, &r.name, rv);
        }
        put(&mut out, "roles", roles);
    }
    if !q.vars.is_empty() {
        let mut vars = Table::new();
        for v in &q.vars {
            put(&mut vars, &v.name, v.value.clone());
        }
        put(&mut out, "vars", vars);
    }
    if !q.tasks.is_empty() {
        let mut tasks = Table::new();
        for t in &q.tasks {
            let mut tv = Table::new();
            if !t.title.is_nil() {
                put(&mut tv, "title", t.title.clone());
            }
            if !t.descr.is_nil() {
                put(&mut tv, "descr", t.descr.clone());
            }
            put(&mut tv, "type", if t.ty.is_empty() { "additional" } else { t.ty.as_str() });
            if !t.target.is_nil() {
                put(&mut tv, "target", t.target.clone());
            }
            if !t.icon.is_empty() {
                put(&mut tv, "icon", t.icon.as_str());
            }
            // closing itself on the last step is the default: only a task that must not says so
            if !t.auto_complete && !t.objectives.is_empty() {
                put(&mut tv, "auto_complete", false);
            }
            if !t.objectives.is_empty() {
                let mut objs = Table::new();
                for o in &t.objectives {
                    let mut ov = Table::new();
                    put(&mut ov, "id", o.id.as_str());
                    if !o.title.is_nil() {
                        put(&mut ov, "title", o.title.clone());
                    }
                    if !o.descr.is_nil() {
                        put(&mut ov, "descr", o.descr.clone());
                    }
                    if !o.target.is_nil() {
                        put(&mut ov, "target", o.target.clone());
                    }
                    if !o.visible {
                        put(&mut ov, "visible", false);
                    }
                    objs.push(ov);
                }
                put(&mut tv, "objectives", objs);
            }
            put(&mut tasks, &t.id, tv);
        }
        put(&mut out, "tasks", tasks);
    }
    let auto_placed = |n: &Node| auto.get(&n.id).is_some_and(|p| *p == n.pos);
    let placed = q.nodes.iter().any(|n| n.has_pos && !auto_placed(n));
    if let Some(c) = q.canvas.to_value(placed) {
        put(&mut out, "canvas", c);
    }
    let node_value = |n: &Node| {
        let mut v = node_to_value(n);
        if auto_placed(n) {
            v.erase("pos");
        }
        v
    };
    let nodes: Vec<Value> = if topo {
        q.topo_order(catalog).iter().filter_map(|&i| q.nodes.get(i)).map(node_value).collect()
    } else {
        q.nodes.iter().map(node_value).collect()
    };
    put(&mut out, "nodes", Value::from(nodes));
    out
}

/// The text of a new file: `return { ... }` in the canonical layout, the header key first, the
/// nodes in their topological order. An existing file is never written this way: a save writes
/// only what changed into the author's text.
#[must_use]
pub fn write(catalog: &Catalog, q: &Graph) -> String {
    gz_luadata::write_asset("", &graph_to_value(catalog, q))
}

/// How a graph file's values map onto its text: nodes matched by `id` and laid out as nodes,
/// task objectives by `id`, new keys in the canonical order.
#[must_use]
pub fn diff_rules() -> gz_luadata::DiffRules {
    gz_luadata::DiffRules::new()
        .keyed("nodes", "id")
        .node_layout("nodes")
        .keyed("tasks.*.objectives", "id")
        .keyed("canvas.bookmarks", "name")
        .order("", &["nq", "beh", "id", "title", "activation", "roles", "vars", "tasks", "canvas", "nodes"])
        .order(
            "nodes.*",
            &[
                "id",
                "kind",
                "once",
                "disabled",
                "params",
                "cond",
                "on_enter",
                "on_exit",
                "out",
                "pos",
                "collapsed",
                "comment",
            ],
        )
        .order("canvas", &["flow", "wires", "frames", "routes", "bookmarks"])
        .order("canvas.wires", &["style", "diagonal", "thickness", "by_pin", "dim", "bubbles", "arrows"])
        .order("canvas.frames.*", &["title", "tone", "x", "y", "w", "h"])
        .order("tasks.*", &["title", "descr", "type", "target", "icon", "auto_complete", "objectives"])
        .order("roles.*", &["title", "many", "optional", "default"])
}

// ---- outline ----------------------------------------------------------------------------------

fn params_summary(params: &Value, skip_text: bool) -> String {
    let mut s = String::new();
    if !params.is_table() {
        return s;
    }
    for (k, v) in params.fields() {
        if skip_text && k == "text" {
            continue;
        }
        if !s.is_empty() {
            s.push_str(", ");
        }
        s.push_str(k);
        s.push('=');
        if k == "code" {
            s.push_str("lua");
        } else if k == "cases" && v.is_table() {
            for (c, case) in v.arr().iter().enumerate() {
                if c > 0 {
                    s.push('|');
                }
                s.push_str(&case.get_string("name", "?"));
            }
        } else if let Value::String(st) = v {
            s.push_str(&gz_luadata::one_line(&text::clip_utf8(st, 40)));
        } else {
            s.push_str(&gz_luadata::one_line(&text::value_summary(v, 40)));
        }
    }
    s
}

fn actions_summary(list: &[Action]) -> String {
    list.iter()
        .map(|a| format!("{}({})", a.kind, params_summary(&a.params, false)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn conds_summary(list: &[Cond]) -> String {
    list.iter()
        .map(|c| {
            format!("{}{}({})", if c.negate { "not " } else { "" }, c.kind, params_summary(&c.params, false))
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// One outline line of a node.
#[must_use]
pub fn outline_node(catalog: &Catalog, n: &Node) -> String {
    let trig = catalog.is_trigger(&n.kind);
    let mut line = String::new();
    if trig {
        line.push_str("[T] ");
    }
    line.push_str(&n.id);
    line.push_str(" (");
    line.push_str(&n.kind);
    let ps = params_summary(&n.params, true);
    if !ps.is_empty() {
        line.push_str("; ");
        line.push_str(&ps);
    }
    if n.once_set {
        line.push_str(if n.once { "; once" } else { "; once=false" });
    }
    if n.disabled {
        line.push_str("; disabled");
    }
    line.push(')');
    if let Some(t) = n.params.get("text") {
        line.push_str(" \"");
        line.push_str(&gz_luadata::one_line(&preview(t)));
        line.push('"');
    }
    if !n.cond.is_empty() {
        line.push_str("  cond: ");
        line.push_str(&conds_summary(&n.cond));
    }
    if !n.on_enter.is_empty() {
        line.push_str("  on_enter: ");
        line.push_str(&actions_summary(&n.on_enter));
    }
    if !n.on_exit.is_empty() {
        line.push_str("  on_exit: ");
        line.push_str(&actions_summary(&n.on_exit));
    }
    let mut any = false;
    for p in &n.out {
        if p.targets.is_empty() {
            continue;
        }
        any = true;
        if trig && n.out.len() == 1 {
            line.push_str(" -> ");
        } else {
            line.push_str("  ");
            line.push_str(&p.name);
            line.push_str(" -> ");
        }
        line.push_str(&p.targets.iter().map(SmolStr::as_str).collect::<Vec<_>>().join(", "));
    }
    if !any {
        line.push_str("  (leaf)");
    }
    line
}

/// The outline: the document line, then one line per node in topological order.
/// A behaviour graph has no activation and no tasks, and never says "quest".
#[must_use]
pub fn outline(catalog: &Catalog, q: &Graph) -> String {
    let behaviour = q.is_behaviour();
    let mut out = String::from(if behaviour { "behaviour " } else { "quest " });
    out.push_str(&q.id);
    out.push_str(" \"");
    out.push_str(&gz_luadata::one_line(&preview(&q.title)));
    out.push('"');
    let mut props = String::new();
    if !behaviour {
        props = format!("active: {}", q.activation);
    }
    if behaviour && !q.roles.is_empty() {
        props.push_str("roles: ");
        props.push_str(&q.roles.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "));
    }
    if !q.vars.is_empty() {
        if !props.is_empty() {
            props.push_str("; ");
        }
        props.push_str("vars: ");
        props.push_str(&q.vars.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", "));
    }
    if !behaviour && !q.tasks.is_empty() {
        props.push_str("; tasks: ");
        props.push_str(&q.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", "));
    }
    if !props.is_empty() {
        out.push_str(" (");
        out.push_str(&props);
        out.push(')');
    }
    out.push('\n');
    for i in q.topo_order(catalog) {
        if let Some(n) = q.nodes.get(i) {
            out.push_str(&outline_node(catalog, n));
            out.push('\n');
        }
    }
    out
}

// ---- templates --------------------------------------------------------------------------------

fn start_finish() -> Vector<Node> {
    let mut start = Node::new("start", "trigger.start");
    start.connect("next", "finish");
    let mut finish = Node::new("finish", "flow.end");
    finish.params.set("status", "completed");
    let mut v = Vector::new();
    v.push_back(start);
    v.push_back(finish);
    v
}

/// What a new quest starts as: `trigger.start -> flow.end`, titled "New quest".
#[must_use]
pub fn quest_template(id: &str) -> String {
    let q = Graph {
        id: if id.is_empty() { "new_quest".to_owned() } else { id.to_owned() },
        title: Value::from("New quest"),
        nodes: start_finish(),
        ..Graph::default()
    };
    write(&Catalog::empty(), &q)
}

/// What a new behaviour graph starts as: one role `squad`, `trigger.start -> flow.end`.
#[must_use]
pub fn behaviour_template(id: &str) -> String {
    let q = Graph {
        id: if id.is_empty() { "new_behaviour".to_owned() } else { id.to_owned() },
        kind: DocKind::Behaviour,
        title: Value::from("New behaviour"),
        roles: vec![Role {
            name: "squad".to_owned(),
            title: Value::from("Squad"),
            many: true,
            ..Role::default()
        }],
        nodes: start_finish(),
        ..Graph::default()
    };
    write(&Catalog::empty(), &q)
}

/// The template of a file name: the id is what the name gives (a quest or a behaviour template).
#[must_use]
pub fn template_for(path: &str) -> String {
    let id = text::id_from_name(&base_name(path));
    if is_behaviour_path(path) { behaviour_template(&id) } else { quest_template(&id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> Catalog {
        let bytes = include_bytes!("../../../gamedata/configs/nq/catalog.ltx");
        let text = gz_base::text::cp1251_to_utf8(bytes).into_owned();
        Catalog::assemble(
            &[crate::catalog::SourceFile { label: "bundled:configs\\nq\\catalog.ltx".into(), text }],
            false,
            None,
            1,
        )
    }

    const EXAMPLES: [(&str, &str); 3] = [
        ("dialog_branching", include_str!("../../../docs/nq/examples/dialog_branching.nqasset")),
        ("linear_fetch", include_str!("../../../docs/nq/examples/linear_fetch.nqasset")),
        ("parallel_triggers", include_str!("../../../docs/nq/examples/parallel_triggers.nqasset")),
    ];

    #[test]
    fn examples_save_only_what_changed() {
        let cat = bundled();
        for (name, text) in EXAMPLES {
            let mut doc = gz_luadata::LuaDoc::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let (g, shape) = graph_from_value(&cat, &doc.value());
            assert!(shape.is_empty(), "{name}: {shape:?}");
            assert_eq!(g.id, name);
            let base = model_value(&cat, &g, &AutoPos::default());
            assert!(
                !doc.apply_diff(&base, &base.clone(), &diff_rules()).unwrap(),
                "{name}: nothing to write"
            );
            // a node moved and a new one linked from the start: the rest of the file stays
            let mut edited = g.clone();
            let start = edited.nodes.iter().position(|n| n.kind == "trigger.start").unwrap();
            let mut fresh = Node::new("checked", "flow.end");
            fresh.params.set("status", "completed");
            edited.nodes.push_back(fresh);
            if let Some(n) = edited.nodes.get_mut(start) {
                n.connect("alt", "checked");
                n.pos = [10.0, 20.0];
                n.has_pos = true;
            }
            let new = model_value(&cat, &edited, &AutoPos::default());
            assert!(doc.apply_diff(&base, &new, &diff_rules()).unwrap());
            let out = doc.text();
            for line in text.lines().filter(|l| l.trim_start().starts_with("--")) {
                assert!(out.contains(line), "{name}: the comment {line:?} went");
            }
            let again = parse(&cat, out, name).unwrap_or_else(|e| panic!("{name}: {}", e.message));
            assert_eq!(again.nodes.len(), g.nodes.len() + 1);
            assert!(again.nodes.iter().any(|n| n.id == "checked"));
            assert_eq!(again.nodes[start].targets("alt"), Some(&[SmolStr::new("checked")][..]));
        }
    }

    #[test]
    fn the_canvas_round_trips_with_its_comments_and_unknown_keys() {
        let cat = bundled();
        let text = r#"return {
  nq = 1,
  id = "ambush",
  canvas = {
    flow = "right", -- the author's
    wires = { style = "circuit", glow = 3 },
    frames = { { title = "The ambush", tone = 3, x = 120, y = 40, w = 620, h = 300 } },
    routes = { ["wait.done>ambush"] = { { 400, 120 }, { 400, 260 } } },
    bookmarks = { { name = "Finale", node = "ambush" } },
    lens = "wide",
  },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "wait" }, pos = { 0, 0 } },
    { id = "wait", kind = "wait.timer", params = { duration = { seconds = 6 } }, collapsed = true, out = { done = "ambush" }, pos = { 240, 0 } },
    { id = "ambush", kind = "flow.step", disabled = true, pos = { 480, 0 }, colour = "red" },
  },
}
"#;
        let mut doc = gz_luadata::LuaDoc::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let (g, shape) = graph_from_value(&cat, &doc.value());
        assert!(shape.is_empty(), "{shape:?}");
        assert_eq!(g.flow(), Flow::Right);
        assert_eq!(g.canvas.wires.style, Some(crate::canvas::WireStyle::Circuit));
        assert_eq!(g.canvas.frames.first().map(|f| f.title.as_str()), Some("The ambush"));
        assert_eq!(g.canvas.route("wait", "done", "ambush").map(<[[f32; 2]]>::len), Some(2));
        assert_eq!(g.canvas.bookmarks.first().map(|b| b.node.as_str()), Some("ambush"));
        assert!(g.find_node("wait").is_some_and(|n| n.collapsed));
        assert!(g.find_node("ambush").is_some_and(|n| n.disabled));
        let base = model_value(&cat, &g, &AutoPos::default());
        // nothing changed: nothing is written
        assert!(!doc.apply_diff(&base, &base.clone(), &diff_rules()).unwrap_or(true));
        assert_eq!(doc.text(), text);
        // a frame moved, a reroute moved and a bookmark renamed: those values change, the rest stays
        let mut edited = g.clone();
        if let Some(f) = edited.canvas.frames.first_mut() {
            f.x = 136.0;
        }
        edited.canvas.set_route("wait", "done", "ambush", vec![[400.0, 120.0], [432.0, 260.0]]);
        if let Some(b) = edited.canvas.bookmarks.first_mut() {
            b.name = "The end".into();
        }
        let new = model_value(&cat, &edited, &AutoPos::default());
        assert!(doc.apply_diff(&base, &new, &diff_rules()).unwrap_or(false));
        let out = doc.text();
        for keep in
            ["-- the author's", "glow = 3", "lens = \"wide\"", "colour = \"red\"", "x = 136", "{ 432, 260 }"]
        {
            assert!(
                out.contains(keep),
                "{keep} went:
{out}"
            );
        }
        // only the lines of what changed changed
        let changed = text.lines().zip(out.lines()).filter(|(a, b)| a != b).count();
        assert_eq!(changed, 3, "{out}");
        let again = parse(&cat, out, "ambush").unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(again.canvas.bookmarks.first().map(|b| b.name.as_str()), Some("The end"));
    }

    #[test]
    fn a_new_graph_writes_its_flow_once_its_nodes_are_placed() {
        let cat = Catalog::empty();
        let (mut g, _) = graph_from_value(
            &cat,
            &parse_value(
                "return { nq = 1, id = 'q', nodes = { { id = 'a', kind = 'trigger.start' } } }",
                "q",
            )
            .unwrap_or_default(),
        );
        // a file without positions takes the preference: here, down
        g.canvas.implied = Flow::Down;
        assert!(model_value(&cat, &g, &AutoPos::default()).get("canvas").is_none());
        g.canvas.implied = Flow::Right;
        let mut auto = AutoPos::default();
        auto.insert("a".into(), [0.0, 0.0]);
        g.nodes[0].has_pos = true;
        // placed by the layout alone: still nothing to write
        assert!(model_value(&cat, &g, &auto).get("canvas").is_none());
        // the author moved it: the flow it was laid out in is written with it
        let v = model_value(&cat, &g, &AutoPos::default());
        assert_eq!(v.get("canvas").map(|c| c.get_string("flow", "")), Some("right".to_owned()));
    }

    #[test]
    fn a_save_keeps_auto_positions_out() {
        let cat = Catalog::empty();
        let (mut g, _) = graph_from_value(
            &cat,
            &parse_value(
                "return { nq = 1, id = 'q', nodes = { { id = 'a', kind = 'trigger.start' } } }",
                "q",
            )
            .unwrap(),
        );
        let base = model_value(&cat, &g, &AutoPos::default());
        if let Some(n) = g.nodes.get_mut(0) {
            n.pos = [3.0, 4.0];
            n.has_pos = true;
        }
        let mut auto = AutoPos::default();
        auto.insert("a".into(), [3.0, 4.0]);
        assert!(gz_luadata::data_eq(&model_value(&cat, &g, &auto), &base));
        assert!(!gz_luadata::data_eq(&model_value(&cat, &g, &AutoPos::default()), &base));
    }

    #[test]
    fn shape_errors_name_what_was_dropped() {
        let cat = Catalog::empty();
        let (g, errs) = graph_from_value(
            &cat,
            &match parse_value(
                "return { nq = 1, id = 'q', type = 'x', nodes = { { kind = 'a' }, { id = 'b', out = { next = { 1 } } } } }",
                "q",
            ) {
                Ok(v) => v,
                Err(e) => panic!("{}", e.message),
            },
        );
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(
            errs,
            [
                "there is no type key: .nqasset + nq is a quest, .behasset + beh is a behaviour graph",
                "node without id (nodes #1)",
                "node 'b': out.next: target #1 is not a node id",
            ]
        );
        let (_, errs) = graph_from_value(&cat, &Value::table());
        assert_eq!(errs, ["nodes list is missing"]);
    }

    #[test]
    fn topo_order_keeps_a_dialog_together() {
        let cat = Catalog::empty();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "topic");
        let mut t = Node::new("topic", "dialog.topic");
        t.connect("next", "p1");
        let mut p1 = Node::new("p1", "dialog.npc_phrase");
        p1.connect("next", "world");
        p1.connect("next", "p2");
        let p2 = Node::new("p2", "dialog.actor_phrase");
        let w = Node::new("world", "flow.end");
        let lone = Node::new("lone", "flow.step");
        for n in [lone, w, p2, p1, t, s] {
            g.nodes.push_front(n);
        }
        let ids: Vec<&str> =
            g.topo_order(&cat).iter().filter_map(|&i| g.nodes.get(i)).map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["s", "topic", "p1", "p2", "world", "lone"]);
    }

    #[test]
    fn templates() {
        let q = quest_template("wolf");
        assert!(q.starts_with("return {\n  nq = 1,\n  id = \"wolf\",\n  title = \"New quest\",\n"), "{q}");
        assert!(q.contains(
            "    { id = \"finish\", kind = \"flow.end\", params = { status = \"completed\" } },\n"
        ));
        let b = behaviour_template("");
        assert!(b.starts_with("return {\n  beh = 1,\n  id = \"new_behaviour\",\n"), "{b}");
        assert!(b.contains("  roles = { squad = { title = \"Squad\", many = true } },\n"));
        assert!(template_for("q/NewBehaviour2.behasset").contains("id = \"newbehaviour2\""));
    }

    #[test]
    fn pins_follow_the_catalog() {
        let cat = bundled();
        let mut n = Node::new("w", "trigger.when");
        n.connect("off", "a");
        n.connect("next", "b");
        n.connect("zz", "c");
        n.connect("aa", "d");
        canonicalize_pins(&cat, &mut n);
        let names: Vec<&str> = n.out.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["next", "off", "aa", "zz"]);
    }
}
