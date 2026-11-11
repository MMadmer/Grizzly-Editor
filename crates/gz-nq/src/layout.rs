//! The node box and the auto layout (`NqLayout`).
//!
//! [`node_box`] is the ONE source of a node's rectangle: the layout pass, the canvas (drawing and
//! hit tests), the minimap and `quest_layout` all read it, so a node is never drawn taller than
//! the layout made room for. It is sized by what the node holds (how many lines, actions and
//! pins), not by measured text, exactly as the C++ sizes it. Units are world pt (pt at zoom 1).

use crate::asset::{Graph, Node};
use crate::catalog::Catalog;
use crate::text;

pub const NODE_WIDTH: f32 = 240.0;
/// The family strip, icon, title, id, badges.
pub const HEADER: f32 = 28.0;
/// One `key: value` line.
pub const LINE: f32 = 20.0;
pub const MAX_LINES: usize = 3;
/// One `on_enter` / `on_exit` action.
pub const ACTION_ROW: f32 = 20.0;
/// Then one `+N more` row.
pub const MAX_ACTIONS: usize = 4;
/// The pin labels, inside the bottom edge.
pub const PIN_ROW: f32 = 18.0;
/// Above a block of lines or actions, and at the bottom.
pub const PAD: f32 = 6.0;
pub const GAP_X: f32 = 64.0;
pub const TREE_GAP_X: f32 = 160.0;
pub const GAP_Y: f32 = 72.0;
pub const GRID: f32 = 16.0;
/// World coordinates are bounded: a corrupt file can carry inf / NaN / 1e18.
pub const WORLD_MAX: f32 = 1.0e6;

/// Finite and inside ±[`WORLD_MAX`]; non-finite becomes 0 (`Sane`).
#[must_use]
pub fn sane(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    v.clamp(-WORLD_MAX, WORLD_MAX)
}

/// On the 16 pt grid.
#[must_use]
pub fn snap(v: f32) -> f32 {
    (v / GRID + 0.5).floor() * GRID
}

/// One line of a node (`SLine`): a condition reads `<title> <what it checks>`, a parameter
/// `<name>: <value>`, the last one `+N more` (no key) when there are more than fit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub key: String,
    pub value: String,
    /// A condition line: the condition's kind.
    pub cond: String,
    pub negate: bool,
}

/// Where the parts of a node sit, in world units from its top-left corner (`SBox`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeBox {
    pub w: f32,
    pub h: f32,
    /// `key: value` lines shown (<= [`MAX_LINES`]).
    pub lines: usize,
    pub lines_y: f32,
    /// `on_enter` + `on_exit`.
    pub actions: usize,
    /// Rows shown: `min(actions, MAX_ACTIONS)`.
    pub action_rows: usize,
    /// A `+N more` row follows them.
    pub more: bool,
    pub actions_y: f32,
    /// The pins are named inside the node.
    pub pin_labels: bool,
    pub pins_y: f32,
}

impl Default for NodeBox {
    fn default() -> Self {
        Self {
            w: NODE_WIDTH,
            h: HEADER + PAD,
            lines: 0,
            lines_y: 0.0,
            actions: 0,
            action_rows: 0,
            more: false,
            actions_y: 0.0,
            pin_labels: false,
            pins_y: 0.0,
        }
    }
}

fn line_value(p: &crate::catalog::Param, v: &gz_luadata::Value) -> String {
    use gz_luadata::Value;
    if p.ty == "text" {
        return text::preview(v);
    }
    if p.ty == "lua" {
        return "Lua".to_owned();
    }
    match v {
        Value::String(s) => s.to_string(),
        Value::Bool(b) => (if *b { "yes" } else { "no" }).to_owned(),
        Value::Number(_) => v.to_text(""),
        Value::Table(_) => text::value_summary(v, 64),
        Value::Nil => String::new(),
    }
}

fn cond_title(catalog: &Catalog, kind: &str) -> String {
    catalog.find(kind).filter(|k| !k.title.is_empty()).map_or_else(|| kind.to_owned(), |k| k.title.clone())
}

fn cond_value(catalog: &Catalog, c: &crate::asset::Cond) -> String {
    let Some(kind) = catalog.find(&c.kind) else {
        let sum = text::value_summary(&c.params, 64);
        return if sum == "{}" { String::new() } else { sum };
    };
    let mut out = String::new();
    for p in &kind.params {
        let Some(v) = c.params.get(&p.name) else { continue };
        if v.is_nil() || v.is_empty() {
            continue;
        }
        let part = if p.ty == "cond_list" && v.is_table() {
            v.arr()
                .iter()
                .map(|e| cond_title(catalog, &e.get_string("kind", "")))
                .collect::<Vec<_>>()
                .join(" or ")
        } else if let Some(b) = v.as_bool() {
            if b { p.name.clone() } else { String::new() }
        } else {
            line_value(p, v)
        };
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&part);
    }
    out
}

/// The key of a condition line: its title, `Not has money` when inverted (an acronym keeps its
/// capitals: `Not NPC alive`).
#[must_use]
pub fn cond_key(title: &str, negate: bool) -> String {
    if !negate {
        return title.to_owned();
    }
    let b = title.as_bytes();
    let mut key = title.to_owned();
    if b.len() > 1
        && b.first().is_some_and(u8::is_ascii_uppercase)
        && !b.get(1).is_some_and(u8::is_ascii_uppercase)
    {
        key.replace_range(..1, &title.get(..1).unwrap_or("").to_ascii_lowercase());
    }
    format!("Not {key}")
}

/// The output pins in draw order - the catalog's (a case node's cases first), then the ones only
/// the file has - and how many of them the catalog declares (`Pins`).
#[must_use]
pub fn pins(catalog: &Catalog, n: &Node) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::new();
    if let Some(kind) = catalog.find(&n.kind) {
        if kind.pins_from_cases
            && let Some(cases) = n.params.get("cases").filter(|c| c.is_table())
        {
            for c in cases.arr() {
                let name = c.get_string("name", "");
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
        for p in &kind.pins {
            if p != "cases" {
                out.push(p.clone());
            }
        }
    }
    let declared = out.len();
    for p in &n.out {
        if !out.iter().any(|x| x == p.name.as_str()) {
            out.push(p.name.to_string());
        }
    }
    (out, declared)
}

/// The lines a node shows, at most [`MAX_LINES`]: its conditions first, then its parameters
/// (`Lines`).
#[must_use]
pub fn lines(catalog: &Catalog, n: &Node) -> Vec<Line> {
    let mut out: Vec<Line> = n
        .cond
        .iter()
        .map(|c| Line {
            key: cond_key(&cond_title(catalog, &c.kind), c.negate),
            value: cond_value(catalog, c),
            cond: c.kind.to_string(),
            negate: c.negate,
        })
        .collect();
    if let Some(kind) = catalog.find(&n.kind) {
        for p in &kind.params {
            // the case names are the pins already
            if p.ty == "cases_cond" || p.ty == "cases_weight" {
                continue;
            }
            let Some(v) = n.params.get(&p.name) else { continue };
            if v.is_nil() || v.is_empty() {
                continue;
            }
            let value = line_value(p, v);
            if !value.is_empty() {
                out.push(Line { key: p.name.clone(), value, ..Line::default() });
            }
        }
    }
    if out.len() > MAX_LINES {
        let hidden = out.len() - (MAX_LINES - 1);
        out.truncate(MAX_LINES - 1);
        out.push(Line { value: format!("+{hidden} more"), ..Line::default() });
    }
    out
}

/// The content-sized box of a node (`Box`).
#[must_use]
pub fn node_box(catalog: &Catalog, n: &Node) -> NodeBox {
    let mut b = NodeBox::default();
    let shown = lines(catalog, n);
    let (pin_list, _) = pins(catalog, n);
    let mut y = HEADER;
    b.lines = shown.len();
    if b.lines > 0 {
        y += PAD;
        b.lines_y = y;
        y += LINE * b.lines as f32;
    }
    // a trigger has no actions to run: the runtime reads them on a main node only
    b.actions = if catalog.is_trigger(&n.kind) { 0 } else { n.on_enter.len() + n.on_exit.len() };
    b.action_rows = b.actions.min(MAX_ACTIONS);
    b.more = b.actions > MAX_ACTIONS;
    if b.actions > 0 {
        y += PAD;
        b.actions_y = y;
        y += ACTION_ROW * (b.action_rows + usize::from(b.more)) as f32;
    }
    // a single "next" pin needs no name
    b.pin_labels =
        pin_list.len() > 1 || (pin_list.len() == 1 && pin_list.first().is_some_and(|p| p != "next"));
    if b.pin_labels {
        y += PAD * 0.5;
        b.pins_y = y;
        y += PIN_ROW;
    } else {
        y += PAD;
    }
    b.h = y;
    b
}

/// The box's size.
#[must_use]
pub fn node_size(catalog: &Catalog, n: &Node) -> [f32; 2] {
    let b = node_box(catalog, n);
    [b.w, b.h]
}

/// Where a node takes its input: the middle of its top edge (`InputPin`).
#[must_use]
pub fn input_pin(pos: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    [pos[0] + size[0] * 0.5, pos[1]]
}

/// Where output pin `pin` of `count` sits: spread along the bottom edge (`OutputPin`).
#[must_use]
pub fn output_pin(pos: [f32; 2], size: [f32; 2], pin: usize, count: usize) -> [f32; 2] {
    [pos[0] + size[0] * (pin + 1) as f32 / (count + 1) as f32, pos[1] + size[1]]
}

/// The bounds of every node box: `[min_x, min_y, max_x, max_y]`, `None` without nodes.
#[must_use]
pub fn graph_bounds(catalog: &Catalog, g: &Graph) -> Option<[f32; 4]> {
    if g.nodes.is_empty() {
        return None;
    }
    let mut b = [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX];
    for n in &g.nodes {
        let s = node_size(catalog, n);
        b[0] = b[0].min(n.pos[0]);
        b[1] = b[1].min(n.pos[1]);
        b[2] = b[2].max(n.pos[0] + s[0]);
        b[3] = b[3].max(n.pos[1] + s[1]);
    }
    Some(b)
}

/// Clamps every node position; the number it had to fix (`SanePositions`).
pub fn sane_positions(g: &mut Graph) -> usize {
    let mut fixed = 0;
    for n in g.nodes.iter_mut() {
        let (x, y) = (sane(n.pos[0]), sane(n.pos[1]));
        // NaN compares unequal to itself, so this catches it too
        #[allow(clippy::float_cmp)]
        let same = x == n.pos[0] && y == n.pos[1];
        if !same {
            n.pos = [x, y];
            fixed += 1;
        }
    }
    fixed
}

/// Clamps positions and lays out only when a node has none (`EnsurePositions`).
pub fn ensure_positions(catalog: &Catalog, g: &mut Graph) -> usize {
    sane_positions(g);
    if g.nodes.iter().any(|n| !n.has_pos) { run(catalog, g, true) } else { 0 }
}

struct Edge {
    from: usize,
    to: usize,
    side: f32,
    order: f32,
    active: bool,
}

#[derive(Clone, Default)]
struct LNode {
    layer: usize,
    parent: usize,
    file_order: usize,
    side_sum: f32,
    side_count: u32,
    order_key: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    center_x: f32,
    offset_x: f32,
    min_x: f32,
    max_x: f32,
    children: Vec<usize>,
}

#[derive(Clone, Copy, Default)]
struct Block {
    min_x: f32,
    max_x: f32,
}

fn pin_side(index: usize, count: usize) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    2.0 * (index + 1) as f32 / (count + 1) as f32 - 1.0
}

fn pack_block(nodes: &mut [LNode], members: &[usize], gap: f32) -> Block {
    let mut cursor = 0.0f32;
    for (i, &m) in members.iter().enumerate() {
        let Some(child) = nodes.get_mut(m) else { continue };
        child.offset_x = cursor - child.min_x;
        cursor += child.max_x - child.min_x;
        if i + 1 < members.len() {
            cursor += gap;
        }
    }
    Block { min_x: 0.0, max_x: cursor }
}

fn shift_block(nodes: &mut [LNode], members: &[usize], block: &mut Block, shift: f32) {
    for &m in members {
        if let Some(n) = nodes.get_mut(m) {
            n.offset_x += shift;
        }
    }
    block.min_x += shift;
    block.max_x += shift;
}

fn pack_roots(nodes: &mut [LNode], roots: &[usize]) {
    if roots.is_empty() {
        return;
    }
    let mut cursor = 0.0f32;
    for (i, &r) in roots.iter().enumerate() {
        let Some(root) = nodes.get_mut(r) else { continue };
        root.center_x = cursor - root.min_x;
        cursor += root.max_x - root.min_x;
        if i + 1 < roots.len() {
            cursor += TREE_GAP_X;
        }
    }
    let shift = -cursor * 0.5;
    for &r in roots {
        if let Some(n) = nodes.get_mut(r) {
            n.center_x += shift;
        }
    }
}

fn branch_under(ancestor: usize, node: usize, nodes: &[LNode]) -> Option<usize> {
    if node == ancestor {
        return None;
    }
    let mut current = node;
    let mut guard = 0usize;
    while let Some(n) = nodes.get(current) {
        if n.parent == usize::MAX || n.parent == ancestor || guard > nodes.len() {
            break;
        }
        current = n.parent;
        guard += 1;
    }
    (nodes.get(current).map(|n| n.parent) == Some(ancestor)).then_some(current)
}

/// A set of node indices as bits: the dominators of one node.
#[derive(Clone)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: usize) {
        if let Some(w) = self.0.get_mut(i / 64) {
            *w |= 1 << (i % 64);
        }
    }
    fn and(&mut self, o: &Bits) {
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a &= *b;
        }
    }
    fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(wi, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = w.trailing_zeros() as usize;
                w &= w - 1;
                Some(wi * 64 + b)
            })
        })
    }
}

/// The deterministic top-down layout (`Run`): a pin-aware dominator tree keeps whole branches
/// apart and puts a node several branches share into a merge block between them; separate
/// trigger trees keep a wider gap. `only_missing` keeps the nodes that have a place and puts the
/// others below them. Returns how many nodes moved.
pub fn run(catalog: &Catalog, q: &mut Graph, only_missing: bool) -> usize {
    let count = q.nodes.len();
    if count == 0 {
        return 0;
    }
    sane_positions(q);
    let idx: rustc_hash::FxHashMap<String, usize> = {
        let mut m = rustc_hash::FxHashMap::default();
        for (i, n) in q.nodes.iter().enumerate() {
            m.entry(n.id.to_string()).or_insert(i);
        }
        m
    };
    let triggers: Vec<bool> = q.nodes.iter().map(|n| catalog.is_trigger(&n.kind)).collect();
    let is_trigger = |i: usize| triggers.get(i).copied().unwrap_or(false);

    let virtual_root = count;
    let mut layout: Vec<LNode> = (0..=count)
        .map(|i| {
            let mut n = LNode { parent: usize::MAX, file_order: i, order_key: i as f32, ..LNode::default() };
            if let Some(node) = q.nodes.get(i) {
                let s = node_size(catalog, node);
                n.w = s[0];
                n.h = s[1];
                n.min_x = -s[0] * 0.5;
                n.max_x = s[0] * 0.5;
            }
            n
        })
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (si, source) in q.nodes.iter().enumerate() {
        let (pin_list, _) = pins(catalog, source);
        for (vp, pin) in pin_list.iter().enumerate() {
            let Some(output) = source.out.iter().find(|p| p.name.as_str() == pin) else { continue };
            for (to_order, t) in output.targets.iter().enumerate() {
                let Some(&target) = idx.get(t.as_str()) else { continue };
                if is_trigger(target) {
                    continue;
                }
                if let Some(o) = outgoing.get_mut(si) {
                    o.push(edges.len());
                }
                edges.push(Edge {
                    from: si,
                    to: target,
                    side: pin_side(vp, pin_list.len()),
                    order: vp as f32 + (to_order + 1) as f32 / (output.targets.len() + 1) as f32,
                    active: false,
                });
            }
        }
    }

    // only DFS back-edges are removed; all other shared paths remain structural
    let mut color = vec![0u8; count];
    let mut finish: Vec<usize> = Vec::with_capacity(count);
    let mut visit = |start: usize, color: &mut Vec<u8>, edges: &mut Vec<Edge>| {
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        if let Some(c) = color.get_mut(start) {
            *c = 1;
        }
        while let Some(&(node, next_edge)) = stack.last() {
            let out = outgoing.get(node).map_or(&[][..], Vec::as_slice);
            if next_edge >= out.len() {
                if let Some(c) = color.get_mut(node) {
                    *c = 2;
                }
                finish.push(node);
                stack.pop();
                continue;
            }
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
            let Some(edge) = out.get(next_edge).and_then(|&e| edges.get_mut(e)) else { continue };
            let to = edge.to;
            let c = color.get(to).copied().unwrap_or(2);
            if c == 1 {
                continue;
            }
            edge.active = true;
            if c != 0 {
                continue;
            }
            if let Some(cc) = color.get_mut(to) {
                *cc = 1;
            }
            stack.push((to, 0));
        }
    };
    for i in 0..count {
        if is_trigger(i) && color.get(i) == Some(&0) {
            visit(i, &mut color, &mut edges);
        }
    }
    for i in 0..count {
        if color.get(i) == Some(&0) {
            visit(i, &mut color, &mut edges);
        }
    }

    let topo: Vec<usize> = finish.iter().rev().copied().collect();
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut unique_parents: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (ei, e) in edges.iter().enumerate() {
        if !e.active {
            continue;
        }
        if let Some(inc) = incoming.get_mut(e.to) {
            inc.push(ei);
        }
        if let Some(up) = unique_parents.get_mut(e.to)
            && !up.contains(&e.from)
        {
            up.push(e.from);
        }
    }

    let mut max_layer = 0usize;
    for &source in &topo {
        let src_layer = layout.get(source).map_or(0, |n| n.layer);
        for &ei in outgoing.get(source).map_or(&[][..], Vec::as_slice) {
            let Some(e) = edges.get(ei) else { continue };
            if !e.active {
                continue;
            }
            if let Some(t) = layout.get_mut(e.to) {
                t.layer = t.layer.max(src_layer + 1);
                max_layer = max_layer.max(t.layer);
            }
        }
    }

    // a DAG dominator tree owns exclusive branches and promotes every join into one merge block
    let mut dominators: Vec<Bits> = vec![Bits::new(count + 1); count + 1];
    let mut dominator_count = vec![0usize; count + 1];
    if let Some(d) = dominators.get_mut(virtual_root) {
        d.set(virtual_root);
    }
    if let Some(c) = dominator_count.get_mut(virtual_root) {
        *c = 1;
    }
    for &ni in &topo {
        let parents = unique_parents.get(ni).map_or(&[][..], Vec::as_slice);
        let mut current = match parents.first() {
            None => dominators.get(virtual_root).cloned().unwrap_or_else(|| Bits::new(count + 1)),
            Some(&p0) => {
                let mut cur = dominators.get(p0).cloned().unwrap_or_else(|| Bits::new(count + 1));
                for &p in parents.iter().skip(1) {
                    if let Some(o) = dominators.get(p) {
                        cur.and(o);
                    }
                }
                cur
            }
        };
        current.set(ni);
        let mut best = virtual_root;
        let mut best_depth = dominator_count.get(virtual_root).copied().unwrap_or(0);
        let mut depth = 0;
        for d in current.ones() {
            depth += 1;
            let dc = dominator_count.get(d).copied().unwrap_or(0);
            if d != ni && dc > best_depth {
                best = d;
                best_depth = dc;
            }
        }
        if let Some(c) = dominator_count.get_mut(ni) {
            *c = depth;
        }
        if let Some(n) = layout.get_mut(ni) {
            n.parent = best;
        }
        if let Some(slot) = dominators.get_mut(ni) {
            *slot = current;
        }
    }

    let mut order_min = vec![1.0e30f32; count];
    let mut order_max = vec![-1.0e30f32; count];
    for e in &edges {
        let single = unique_parents.get(e.to).is_some_and(|p| p.len() == 1);
        if !e.active || !single || layout.get(e.to).map(|n| n.parent) != Some(e.from) {
            continue;
        }
        if let Some(child) = layout.get_mut(e.to) {
            child.side_sum += e.side;
            child.side_count += 1;
        }
        if let (Some(mn), Some(mx)) = (order_min.get_mut(e.to), order_max.get_mut(e.to)) {
            *mn = mn.min(e.order);
            *mx = mx.max(e.order);
        }
    }
    for i in 0..count {
        let single = unique_parents.get(i).is_some_and(|p| p.len() == 1);
        let (mn, mx) = (order_min.get(i).copied().unwrap_or(0.0), order_max.get(i).copied().unwrap_or(0.0));
        if single
            && mn <= mx
            && let Some(n) = layout.get_mut(i)
        {
            n.order_key = (mn + mx) * 0.5;
        }
    }

    let mut root_order = 0.0f32;
    for i in 0..count {
        if is_trigger(i)
            && let Some(n) = layout.get_mut(i)
            && n.parent == virtual_root
        {
            n.order_key = root_order;
            root_order += 1.0;
        }
    }
    for i in 0..count {
        let orphan = unique_parents.get(i).is_some_and(Vec::is_empty);
        if !is_trigger(i)
            && orphan
            && let Some(n) = layout.get_mut(i)
            && n.parent == virtual_root
        {
            n.order_key = root_order;
            root_order += 1.0;
        }
    }

    for &ni in &topo {
        let parents = unique_parents.get(ni).map_or(&[][..], Vec::as_slice);
        if parents.len() <= 1 {
            continue;
        }
        let ancestor = layout.get(ni).map_or(virtual_root, |n| n.parent);
        let mut branches: Vec<usize> = Vec::new();
        let (mut min_key, mut max_key) = (1.0e30f32, -1.0e30f32);
        let (mut min_side, mut max_side) = (1.0e30f32, -1.0e30f32);
        for &parent in parents {
            if let Some(branch) = branch_under(ancestor, parent, &layout) {
                if branches.contains(&branch) {
                    continue;
                }
                branches.push(branch);
                let Some(anchor) = layout.get(branch) else { continue };
                let side =
                    if anchor.side_count > 0 { anchor.side_sum / anchor.side_count as f32 } else { 0.0 };
                min_key = min_key.min(anchor.order_key);
                max_key = max_key.max(anchor.order_key);
                min_side = min_side.min(side);
                max_side = max_side.max(side);
                continue;
            }
            for &ei in incoming.get(ni).map_or(&[][..], Vec::as_slice) {
                let Some(e) = edges.get(ei) else { continue };
                if e.from != ancestor {
                    continue;
                }
                min_key = min_key.min(e.order);
                max_key = max_key.max(e.order);
                min_side = min_side.min(e.side);
                max_side = max_side.max(e.side);
            }
        }
        if let Some(n) = layout.get_mut(ni) {
            if min_key <= max_key {
                n.order_key = (min_key + max_key) * 0.5;
            }
            if min_side <= max_side {
                n.side_sum = (min_side + max_side) * 0.5;
                n.side_count = 1;
            }
        }
    }

    for i in 0..count {
        let parent = layout.get(i).map_or(usize::MAX, |n| n.parent);
        if let Some(p) = layout.get_mut(parent) {
            p.children.push(i);
        }
    }
    let keys: Vec<(f32, usize, usize)> =
        layout.iter().map(|n| (n.order_key, n.layer, n.file_order)).collect();
    for n in &mut layout {
        n.children.sort_by(|&a, &b| {
            let (ka, kb) =
                (keys.get(a).copied().unwrap_or_default(), keys.get(b).copied().unwrap_or_default());
            ka.0.partial_cmp(&kb.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ka.1.cmp(&kb.1))
                .then(ka.2.cmp(&kb.2))
        });
    }

    // children before their dominators, so every packed block uses complete descendant bounds
    for &index in topo.iter().rev() {
        let children = layout.get(index).map(|n| n.children.clone()).unwrap_or_default();
        let (mut left, mut middle, mut right) = (Vec::new(), Vec::new(), Vec::new());
        for &c in &children {
            let Some(child) = layout.get(c) else { continue };
            let side = if child.side_count > 0 { child.side_sum / child.side_count as f32 } else { 0.0 };
            if side < -0.001 {
                left.push(c);
            } else if side > 0.001 {
                right.push(c);
            } else {
                middle.push(c);
            }
        }
        let mut middle_block = Block::default();
        if let [only] = middle.as_slice() {
            if let Some(child) = layout.get_mut(*only) {
                child.offset_x = 0.0;
                middle_block = Block { min_x: child.min_x, max_x: child.max_x };
            }
        } else if !middle.is_empty() {
            middle_block = pack_block(&mut layout, &middle, GAP_X);
            let shift = -(middle_block.min_x + middle_block.max_x) * 0.5;
            shift_block(&mut layout, &middle, &mut middle_block, shift);
        }
        let mut left_block = Block::default();
        if !left.is_empty() {
            left_block = pack_block(&mut layout, &left, GAP_X);
            let edge = if middle.is_empty() { -GAP_X * 0.5 } else { middle_block.min_x - GAP_X };
            let shift = edge - left_block.max_x;
            shift_block(&mut layout, &left, &mut left_block, shift);
        }
        let mut right_block = Block::default();
        if !right.is_empty() {
            right_block = pack_block(&mut layout, &right, GAP_X);
            let edge = if middle.is_empty() { GAP_X * 0.5 } else { middle_block.max_x + GAP_X };
            let shift = edge - right_block.min_x;
            shift_block(&mut layout, &right, &mut right_block, shift);
        }
        let Some(node) = layout.get_mut(index) else { continue };
        node.min_x = -node.w * 0.5;
        node.max_x = node.w * 0.5;
        for (members, block) in [(&left, left_block), (&middle, middle_block), (&right, right_block)] {
            if !members.is_empty() {
                node.min_x = node.min_x.min(block.min_x);
                node.max_x = node.max_x.max(block.max_x);
            }
        }
    }

    let roots = layout.get(virtual_root).map(|n| n.children.clone()).unwrap_or_default();
    pack_roots(&mut layout, &roots);
    for &i in &topo {
        let parent = layout.get(i).map_or(virtual_root, |n| n.parent);
        if parent != virtual_root {
            let pc = layout.get(parent).map_or(0.0, |p| p.center_x);
            if let Some(n) = layout.get_mut(i) {
                n.center_x = pc + n.offset_x;
            }
        }
    }

    let mut row_height = vec![0.0f32; max_layer + 1];
    let mut row_y = vec![0.0f32; max_layer + 1];
    for n in layout.iter().take(count) {
        if let Some(r) = row_height.get_mut(n.layer) {
            *r = r.max(n.h);
        }
    }
    let mut y = 0.0f32;
    for layer in 0..=max_layer {
        let ry = snap(y);
        if let Some(r) = row_y.get_mut(layer) {
            *r = ry;
        }
        y = ry + row_height.get(layer).copied().unwrap_or(0.0) + GAP_Y;
    }
    for n in layout.iter_mut().take(count) {
        n.x = sane(snap(n.center_x - n.w * 0.5));
        n.y = sane(row_y.get(n.layer).copied().unwrap_or(0.0));
    }

    let mut moved = 0;
    if only_missing {
        let mut any_existing = false;
        let (mut max_y, mut min_missing_y) = (-1e9f32, 1e9f32);
        for (n, l) in q.nodes.iter().zip(&layout) {
            if n.has_pos {
                any_existing = true;
                max_y = max_y.max(n.pos[1] + l.h);
            } else {
                min_missing_y = min_missing_y.min(l.y);
            }
        }
        let dy = if any_existing { snap(max_y + GAP_Y - min_missing_y) } else { 0.0 };
        for (n, l) in q.nodes.iter_mut().zip(&layout) {
            if n.has_pos {
                continue;
            }
            n.pos = [l.x, sane(l.y + dy)];
            n.has_pos = true;
            moved += 1;
        }
        return moved;
    }
    for (n, l) in q.nodes.iter_mut().zip(&layout) {
        #[allow(clippy::float_cmp)]
        let same = n.has_pos && n.pos[0] == l.x && n.pos[1] == l.y;
        if !same {
            moved += 1;
        }
        n.pos = [l.x, l.y];
        n.has_pos = true;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, Kind, Param, USE_MAIN, USE_TRIGGER};

    fn catalog() -> Catalog {
        let start = Kind {
            id: "trigger.start".into(),
            uses: USE_TRIGGER,
            pins: vec!["next".into()],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        let topic = Kind {
            id: "dialog.topic".into(),
            uses: USE_MAIN,
            pins: vec!["next".into(), "done".into()],
            params: vec![
                Param { name: "npc".into(), ty: "npc_ref".into(), ..Param::default() },
                Param { name: "text".into(), ty: "text".into(), ..Param::default() },
            ],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        let step = Kind {
            id: "flow.step".into(),
            uses: USE_MAIN,
            pins: vec!["next".into()],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        Catalog::from_kinds(vec![start, topic, step])
    }

    #[test]
    fn the_box_follows_the_design() {
        let cat = catalog();
        // a bare step: header, bottom pad, its single "next" pin unnamed
        let step = Node::new("s", "flow.step");
        assert_eq!(node_box(&cat, &step), NodeBox { h: HEADER + PAD, ..NodeBox::default() });
        assert_eq!(node_size(&cat, &step), [240.0, 34.0]);

        // a topic with two parameters and two pins: 28 + 6 + 2 * 20 + 3 + 18
        let mut topic = Node::new("t", "dialog.topic");
        topic.params.set("npc", gz_luadata::Value::table());
        if let Some(npc) = topic.params.table_mut().and_then(|t| t.get_mut("npc")) {
            npc.set("story", "esc_wolf");
        }
        topic.params.set("text", "Hello");
        let b = node_box(&cat, &topic);
        assert_eq!((b.lines, b.lines_y, b.pin_labels, b.pins_y, b.h), (2, 34.0, true, 77.0, 95.0));
        assert_eq!(lines(&cat, &topic).first().map(|l| l.value.as_str()), Some("story:esc_wolf"));

        // six actions: four rows and "+N more"
        let mut busy = Node::new("b", "flow.step");
        for _ in 0..6 {
            busy.on_enter.push(crate::asset::Action { kind: "x".into(), params: gz_luadata::Value::Nil });
        }
        let b = node_box(&cat, &busy);
        assert_eq!(
            (b.actions, b.action_rows, b.more, b.actions_y, b.h),
            (6, 4, true, 34.0, 34.0 + 100.0 + 6.0)
        );

        // a trigger runs no actions
        let mut t = Node::new("go", "trigger.start");
        t.on_enter.push(crate::asset::Action { kind: "x".into(), params: gz_luadata::Value::Nil });
        assert_eq!(node_box(&cat, &t).actions, 0);
    }

    #[test]
    fn more_lines_than_fit() {
        let cat = Catalog::empty();
        let mut n = Node::new("n", "unknown");
        for k in ["a", "b", "c", "d"] {
            n.cond.push(crate::asset::Cond {
                kind: k.into(),
                params: gz_luadata::Value::Nil,
                negate: k == "b",
            });
        }
        let l = lines(&cat, &n);
        assert_eq!(l.len(), 3);
        assert_eq!(l.get(1).map(|x| x.key.as_str()), Some("Not b"));
        assert_eq!(l.get(2).map(|x| x.value.as_str()), Some("+3 more"));
        assert_eq!(cond_key("Has money", true), "Not has money");
        assert_eq!(cond_key("NPC alive", true), "Not NPC alive");
    }

    #[test]
    fn layout_is_deterministic_and_top_down() {
        let cat = catalog();
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "b");
        let b = Node::new("b", "flow.step");
        g.nodes.push_back(s);
        g.nodes.push_back(a);
        g.nodes.push_back(b);
        let moved = run(&cat, &mut g, false);
        assert_eq!(moved, 3);
        let ys: Vec<f32> = g.nodes.iter().map(|n| n.pos[1]).collect();
        assert_eq!(ys, [0.0, 112.0, 224.0]);
        let xs: Vec<f32> = g.nodes.iter().map(|n| n.pos[0]).collect();
        assert_eq!(xs, [-112.0, -112.0, -112.0]);
        assert_eq!(run(&cat, &mut g, false), 0);
        assert_eq!(sane(f32::NAN), 0.0);
        assert_eq!(sane(1e9), WORLD_MAX);
    }
}
