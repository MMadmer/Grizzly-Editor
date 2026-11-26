//! The node box (`node_box`) and the places of the pins.
//!
//! [`node_box`] is the ONE source of a node's rectangle: the auto layout ([`crate::arrange`]), the
//! canvas (drawing and hit tests), the minimap and `graph.get` all read it, so a node is never drawn
//! larger than the layout made room for. A node is as wide as its title and its widest line need
//! (180-360 pt), measured with the canvas's faces ([`crate::measure`]), and as tall as what it holds.
//! Units are world pt (pt at zoom 1).

use crate::asset::{Graph, Node};
use crate::canvas::Flow;
use crate::catalog::Catalog;
use crate::measure::{self, Face};
use crate::text;

pub const NODE_MIN_W: f32 = 180.0;
pub const NODE_MAX_W: f32 = 360.0;
/// The family icon, the title, the id, the badges.
pub const HEADER: f32 = 28.0;
/// The author's comment, italic, under the header.
pub const COMMENT: f32 = 18.0;
/// One `key: value` line.
pub const LINE: f32 = 20.0;
pub const MAX_LINES: usize = 3;
/// One `on_enter` / `on_exit` action.
pub const ACTION_ROW: f32 = 20.0;
/// Then one `+N more` row.
pub const MAX_ACTIONS: usize = 4;
/// Above a block of lines or actions, and at the bottom.
pub const PAD: f32 = 6.0;
/// Text keeps this far from the card's edges.
pub const INSET: f32 = 8.0;
/// Between two output pins on the right edge of a node that flows right.
pub const PIN_PITCH: f32 = 18.0;
/// The em sizes of the canvas's text: the title, a line or an action, the id and the other
/// muted text (its line box is 16 / 15 / 15 pt).
pub const TITLE_EM: f32 = 12.0;
pub const BODY_EM: f32 = 11.25;
pub const SMALL_EM: f32 = 11.25;
/// The header's icon and an action row's icon.
pub const ICON: f32 = 16.0;
pub const ROW_ICON: f32 = 14.0;
pub const GRID: f32 = 16.0;
/// World coordinates are bounded: a corrupt file can carry inf / NaN / 1e18.
pub const WORLD_MAX: f32 = 1.0e6;

/// Finite and inside ±[`WORLD_MAX`]; non-finite becomes 0.
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

/// What sizing a node reads besides the node: the catalog, the flow and the document's kind (a
/// behaviour graph names its kinds its own way), and how a window that speaks another language
/// than English shows a title and a line's name (`None`: as the catalog says them, in English).
#[derive(Clone, Copy)]
pub struct BoxCx<'a> {
    pub catalog: &'a Catalog,
    pub flow: Flow,
    pub behaviour: bool,
    pub shown: Option<&'a (dyn Fn(Shown<'_>) -> String + Sync)>,
}

/// A text of a node a window shows in its own language.
#[derive(Clone, Copy, Debug)]
pub enum Shown<'s> {
    /// A kind's title: the node's, an action's.
    Title(&'s str),
    /// The name a line starts with: a parameter's, a condition's title.
    Key(&'s Line),
    /// A word of the header's badges ("once", "off").
    Badge(&'s str),
}

impl<'a> BoxCx<'a> {
    #[must_use]
    pub fn of(catalog: &'a Catalog, g: &Graph) -> Self {
        Self { catalog, flow: g.flow(), behaviour: g.is_behaviour(), shown: None }
    }

    fn title(&self, en: String) -> String {
        match self.shown {
            Some(f) => f(Shown::Title(&en)),
            None => en,
        }
    }

    fn key(&self, l: &Line) -> String {
        match self.shown {
            Some(f) => f(Shown::Key(l)),
            None => l.key.clone(),
        }
    }

    fn badge(&self, en: &str) -> String {
        match self.shown {
            Some(f) => f(Shown::Badge(en)),
            None => en.to_owned(),
        }
    }
}

/// The room the header keeps for a problem badge (the canvas draws one while the node has a
/// problem: an icon and a count), so that a box does not change its size as problems come and go.
pub const PROBLEM_BADGE: f32 = 40.0;

/// One line of a node: a condition reads `<title> <what it checks>`, a parameter
/// `<name>: <value>`, the last one `+N more` (no key) when there are more than fit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub key: String,
    pub value: String,
    /// A condition line: the condition's kind.
    pub cond: String,
    pub negate: bool,
}

/// Where the parts of a node sit, in world units from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeBox {
    pub w: f32,
    pub h: f32,
    /// The comment's line, when the node shows one.
    pub comment_y: Option<f32>,
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
    /// Header and pins alone.
    pub collapsed: bool,
}

impl Default for NodeBox {
    fn default() -> Self {
        Self {
            w: NODE_MIN_W,
            h: HEADER + PAD,
            comment_y: None,
            lines: 0,
            lines_y: 0.0,
            actions: 0,
            action_rows: 0,
            more: false,
            actions_y: 0.0,
            collapsed: false,
        }
    }
}

fn cond_title(catalog: &Catalog, kind: &str, behaviour: bool) -> String {
    if catalog.find(kind).is_some_and(|k| !k.title.is_empty()) {
        text::kind_title(catalog, kind, behaviour)
    } else {
        kind.to_owned()
    }
}

fn cond_value(catalog: &Catalog, c: &crate::asset::Cond, behaviour: bool) -> String {
    let Some(kind) = catalog.find(&c.kind) else {
        return text::summary(catalog, &c.kind, &c.params, None);
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
                .map(|e| cond_title(catalog, &e.get_string("kind", ""), behaviour))
                .collect::<Vec<_>>()
                .join(" or ")
        } else if let Some(b) = v.as_bool() {
            if b { p.name.clone() } else { String::new() }
        } else {
            text::param_text(p, v, None)
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
/// the file has - and how many of them the catalog declares.
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

/// The pin a node's flow takes when nothing else happens: `next`, else `done`, else the kind's
/// first pin that is no failure. Its wire carries no label.
#[must_use]
pub fn default_pin(pins: &[String]) -> Option<&str> {
    for want in ["next", "done", "finished"] {
        if let Some(p) = pins.iter().find(|p| p.as_str() == want) {
            return Some(p.as_str());
        }
    }
    pins.iter().find(|p| !is_failure_pin(p)).map(String::as_str)
}

/// A pin that leaves when something went wrong: its wire is drawn in the danger hue.
#[must_use]
pub fn is_failure_pin(pin: &str) -> bool {
    matches!(pin, "timeout" | "fail" | "failed" | "aborted" | "abort" | "error")
}

/// The lines a node shows, at most [`MAX_LINES`]: its conditions first, then its parameters.
#[must_use]
pub fn lines(catalog: &Catalog, n: &Node, behaviour: bool) -> Vec<Line> {
    let mut out: Vec<Line> = n
        .cond
        .iter()
        .map(|c| Line {
            key: cond_key(&cond_title(catalog, &c.kind, behaviour), c.negate),
            value: cond_value(catalog, c, behaviour),
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
            let value = text::param_text(p, v, None);
            if !value.is_empty() {
                out.push(Line { key: text::pretty_lower(&p.name), value, ..Line::default() });
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

/// The width a line needs: `key: value` or `Title what`.
fn line_width(cx: &BoxCx<'_>, l: &Line) -> f32 {
    let name = cx.key(l);
    let key = if l.cond.is_empty() && !l.key.is_empty() { format!("{name}:") } else { name };
    INSET * 2.0
        + measure::width(Face::Body, BODY_EM, &key)
        + 6.0
        + measure::width(Face::Body, BODY_EM, &l.value)
}

/// The content-sized box of a node in a flow.
#[must_use]
pub fn node_box(cx: &BoxCx<'_>, n: &Node) -> NodeBox {
    let cat = cx.catalog;
    let mut b = NodeBox { collapsed: n.collapsed, ..NodeBox::default() };
    let title = cx.title(text::kind_title(cat, &n.kind, cx.behaviour));
    // the header: the icon, the title, the id, and from the right the problem badge and the
    // words "off" and "once" - every one of them as the canvas draws it
    let mut badges = PROBLEM_BADGE + 4.0;
    // "once" as the canvas tells it: the node's own word, else its kind's default
    let once = !cat.is_trigger(&n.kind)
        && if n.once_set { n.once } else { cat.find(&n.kind).is_some_and(|k| k.once_default) };
    for (on, word) in [(n.disabled, "off"), (once, "once")] {
        if on {
            badges += measure::width(Face::Body, SMALL_EM, &cx.badge(word)) + 6.0;
        }
    }
    let mut need = INSET
        + ICON
        + 6.0
        + measure::width(Face::Strong, TITLE_EM, &title)
        + 10.0
        + measure::width(Face::Body, SMALL_EM, &n.id)
        + badges
        + INSET;
    let (pin_list, _) = pins(cat, n);
    let mut y = HEADER;
    if !n.collapsed {
        if !n.comment.is_empty() {
            let first = n.comment.lines().next().unwrap_or("");
            need = need.max(INSET * 2.0 + measure::width(Face::Body, SMALL_EM, first) + 4.0);
            b.comment_y = Some(y);
            y += COMMENT;
        }
        let shown = lines(cat, n, cx.behaviour);
        b.lines = shown.len();
        for l in &shown {
            need = need.max(line_width(cx, l));
        }
        if b.lines > 0 {
            y += PAD;
            b.lines_y = y;
            y += LINE * b.lines as f32;
        }
        // a trigger has no actions to run: the runtime reads them on a main node only
        b.actions = if cat.is_trigger(&n.kind) { 0 } else { n.on_enter.len() + n.on_exit.len() };
        b.action_rows = b.actions.min(MAX_ACTIONS);
        b.more = b.actions > MAX_ACTIONS;
        for (exit, a) in n
            .on_enter
            .iter()
            .map(|a| (false, a))
            .chain(n.on_exit.iter().map(|a| (true, a)))
            .take(b.action_rows)
        {
            let (t, s) = (
                cx.title(text::kind_title(cat, &a.kind, cx.behaviour)),
                text::summary(cat, &a.kind, &a.params, None),
            );
            let words = if s.is_empty() { t } else { format!("{t}: {s}") };
            // every row says the slot it runs in, enter or exit, as the canvas draws it
            let tag =
                measure::width(Face::Body, SMALL_EM, &cx.badge(if exit { "exit" } else { "enter" })) + 6.0;
            need = need.max(INSET * 2.0 + ROW_ICON + 6.0 + measure::width(Face::Body, BODY_EM, &words) + tag);
        }
        if b.actions > 0 {
            y += PAD;
            b.actions_y = y;
            y += ACTION_ROW * (b.action_rows + usize::from(b.more)) as f32;
        }
        y += PAD;
    }
    // the outputs of a node that flows right stack down its right edge
    if cx.flow == Flow::Right && pin_list.len() > 1 {
        y = y.max(HEADER + PIN_PITCH * (pin_list.len() - 1) as f32 + PAD);
    }
    b.w = ((need.clamp(NODE_MIN_W, NODE_MAX_W) / 4.0).ceil() * 4.0).min(NODE_MAX_W);
    b.h = y;
    b
}

/// The box's size.
#[must_use]
pub fn node_size(cx: &BoxCx<'_>, n: &Node) -> [f32; 2] {
    let b = node_box(cx, n);
    [b.w, b.h]
}

/// Where a node takes its input: the middle of its top edge flowing down, beside its header on
/// its left edge flowing right.
#[must_use]
pub fn input_pin(flow: Flow, pos: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    match flow {
        Flow::Down => [pos[0] + size[0] * 0.5, pos[1]],
        Flow::Right => [pos[0], pos[1] + HEADER * 0.5],
    }
}

/// Where output pin `pin` of `count` sits: spread along the bottom edge flowing down, down the
/// right edge from the header flowing right.
#[must_use]
pub fn output_pin(flow: Flow, pos: [f32; 2], size: [f32; 2], pin: usize, count: usize) -> [f32; 2] {
    match flow {
        Flow::Down => [pos[0] + size[0] * (pin + 1) as f32 / (count + 1) as f32, pos[1] + size[1]],
        Flow::Right => [pos[0] + size[0], pos[1] + HEADER * 0.5 + PIN_PITCH * pin as f32],
    }
}

/// The bounds of every node box: `[min_x, min_y, max_x, max_y]`, `None` without nodes.
#[must_use]
pub fn graph_bounds(catalog: &Catalog, g: &Graph) -> Option<[f32; 4]> {
    if g.nodes.is_empty() {
        return None;
    }
    let cx = BoxCx::of(catalog, g);
    let mut b = [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX];
    for n in &g.nodes {
        let s = node_size(&cx, n);
        b[0] = b[0].min(n.pos[0]);
        b[1] = b[1].min(n.pos[1]);
        b[2] = b[2].max(n.pos[0] + s[0]);
        b[3] = b[3].max(n.pos[1] + s[1]);
    }
    Some(b)
}

/// Clamps every node position; the number it had to fix.
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

/// Clamps positions and lays out only when a node has none.
pub fn ensure_positions(catalog: &Catalog, g: &mut Graph) -> usize {
    sane_positions(g);
    if g.nodes.iter().any(|n| !n.has_pos) { run(catalog, g, true) } else { 0 }
}

/// The nodes without a place that a wire joins to a placed one go beside it, where the auto layout
/// would put them relative to it (a start before the node it leads to), when nothing stands there.
/// Returns how many it placed.
fn place_beside(cx: &BoxCx<'_>, g: &mut Graph, placed: &[[f32; 2]]) -> usize {
    let n = g.nodes.len();
    let size: Vec<[f32; 2]> = g.nodes.iter().map(|x| node_size(cx, x)).collect();
    let mut near: Vec<Vec<usize>> = vec![Vec::new(); n];
    {
        let index = g.index();
        for (u, x) in g.nodes.iter().enumerate() {
            for p in &x.out {
                for t in &p.targets {
                    if let Some(&v) = index.get(t.as_str())
                        && v != u
                    {
                        // what a node leads to first: the start of a chain goes before it
                        near[u].insert(0, v);
                        near[v].push(u);
                    }
                }
            }
        }
    }
    let mut at: Vec<Option<[f32; 2]>> = g.nodes.iter().map(|x| x.has_pos.then_some(x.pos)).collect();
    let gap = 8.0;
    let free = |at: &[Option<[f32; 2]>], m: usize, p: [f32; 2]| {
        at.iter().enumerate().all(|(j, q)| {
            let Some(q) = q else { return true };
            j == m
                || p[0] + size[m][0] + gap <= q[0]
                || q[0] + size[j][0] + gap <= p[0]
                || p[1] + size[m][1] + gap <= q[1]
                || q[1] + size[j][1] + gap <= p[1]
        })
    };
    // a chain of new nodes grows from the placed nodes it hangs on: each node is tried once, from
    // the placed neighbours it has when its turn comes, nearest the placed ones first
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&m| at[m].is_none() && near[m].iter().any(|&q| at[q].is_some())).collect();
    let mut queued: Vec<bool> = vec![false; n];
    for &m in &queue {
        queued[m] = true;
    }
    let mut count = 0;
    while let Some(m) = queue.pop_front() {
        let spot = near[m].iter().find_map(|&q| {
            let base = at[q]?;
            let p = [
                sane(snap(base[0] + placed[m][0] - placed[q][0])),
                sane(snap(base[1] + placed[m][1] - placed[q][1])),
            ];
            free(&at, m, p).then_some(p)
        });
        let Some(p) = spot else { continue };
        at[m] = Some(p);
        if let Some(x) = g.nodes.get_mut(m) {
            x.pos = p;
            x.has_pos = true;
        }
        count += 1;
        for &v in &near[m] {
            if at[v].is_none() && !queued[v] {
                queued[v] = true;
                queue.push_back(v);
            }
        }
    }
    count
}

/// The whole graph laid out again (`only_missing`: only the nodes without a place, put beside the
/// others). Returns how many nodes moved.
pub fn run(catalog: &Catalog, g: &mut Graph, only_missing: bool) -> usize {
    let all: Vec<usize> = (0..g.nodes.len()).collect();
    let placed = crate::arrange::layout(catalog, g, &all);
    let mut moved = 0;
    if only_missing {
        let cx = BoxCx::of(catalog, g);
        let moved_beside = place_beside(&cx, g, &placed);
        moved += moved_beside;
        let mut max_y = f32::MIN;
        let mut min_missing = f32::MAX;
        let mut any = false;
        for (n, p) in g.nodes.iter().zip(&placed) {
            if n.has_pos {
                any = true;
                max_y = max_y.max(n.pos[1] + node_size(&cx, n)[1]);
            } else {
                min_missing = min_missing.min(p[1]);
            }
        }
        let dy = if any { snap(max_y + crate::arrange::gaps(cx.flow).depth - min_missing) } else { 0.0 };
        for (n, p) in g.nodes.iter_mut().zip(&placed) {
            if n.has_pos {
                continue;
            }
            n.pos = [p[0], sane(p[1] + dy)];
            n.has_pos = true;
            moved += 1;
        }
        return moved;
    }
    for (n, p) in g.nodes.iter_mut().zip(&placed) {
        #[allow(clippy::float_cmp)]
        let same = n.has_pos && n.pos == *p;
        if !same {
            moved += 1;
        }
        n.pos = *p;
        n.has_pos = true;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Kind, Param, USE_MAIN, USE_TRIGGER};

    fn catalog() -> Catalog {
        let start = Kind {
            id: "trigger.start".into(),
            title: "Quest start".into(),
            uses: USE_TRIGGER,
            pins: vec!["next".into()],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        let topic = Kind {
            id: "dialog.topic".into(),
            title: "Dialog: topic".into(),
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
            title: "Step".into(),
            uses: USE_MAIN,
            pins: vec!["next".into()],
            in_quest: true,
            in_behaviour: true,
            ..Kind::default()
        };
        Catalog::from_kinds(vec![start, topic, step])
    }

    /// A node without a place that leads to a placed one goes before it, where the layout would
    /// have it (the file of a quest whose start was never moved); one that hangs on nothing goes
    /// below the rest.
    #[test]
    fn a_node_without_a_place_goes_beside_what_it_is_wired_to() {
        let cat = catalog();
        let mut g = Graph::default();
        g.canvas.flow = Some(Flow::Down);
        let mut start = Node::new("start", "trigger.start");
        start.connect("next", "topic");
        let mut topic = Node::new("topic", "dialog.topic");
        topic.pos = [-48.0, 144.0];
        topic.has_pos = true;
        let mut far = Node::new("far", "dialog.topic");
        far.pos = [-48.0, 900.0];
        far.has_pos = true;
        for n in [start, topic, far, Node::new("loose", "trigger.start")] {
            g.nodes.push_back(n);
        }
        ensure_positions(&cat, &mut g);
        let cx = BoxCx::of(&cat, &g);
        let (s, t) = (&g.nodes[0], &g.nodes[1]);
        assert!(s.has_pos);
        // above the topic, its wire running down into it
        assert!(s.pos[1] + node_size(&cx, s)[1] <= t.pos[1], "{:?} {:?}", s.pos, t.pos);
        assert!((s.pos[1] - t.pos[1]).abs() < 400.0, "{:?} {:?}", s.pos, t.pos);
        // the loose start hangs on nothing: below everything
        assert!(g.nodes[3].pos[1] > 900.0, "{:?}", g.nodes[3].pos);
    }

    #[test]
    fn the_box_is_sized_by_what_it_says() {
        let cat = catalog();
        let cx = BoxCx { catalog: &cat, flow: Flow::Down, behaviour: false, shown: None };
        // a bare step: header and the bottom pad, the narrowest width
        let step = Node::new("s", "flow.step");
        assert_eq!(node_box(&cx, &step), NodeBox { h: HEADER + PAD, ..NodeBox::default() });
        assert_eq!(node_size(&cx, &step), [NODE_MIN_W, 34.0]);

        // a topic with two parameters: 28 + 6 + 2 * 20 + 6
        let mut topic = Node::new("t", "dialog.topic");
        let mut npc = gz_luadata::Value::table();
        npc.set("story", "esc_wolf");
        topic.params.set("npc", npc);
        topic.params.set("text", "Hello");
        let b = node_box(&cx, &topic);
        assert_eq!((b.lines, b.lines_y, b.h), (2, 34.0, 80.0));
        assert_eq!(lines(&cat, &topic, false).first().map(|l| l.value.as_str()), Some("esc_wolf"));

        // a long text makes it wider, never past 360
        topic.params.set(
            "text",
            "A very long line of text the author wrote into a topic node, far wider than a node",
        );
        let wide = node_box(&cx, &topic);
        assert!(wide.w > b.w && wide.w <= NODE_MAX_W, "{} {}", wide.w, b.w);

        // six actions: four rows and "+N more"
        let mut busy = Node::new("b", "flow.step");
        for _ in 0..6 {
            busy.on_enter.push(crate::asset::Action { kind: "x".into(), params: gz_luadata::Value::Nil });
        }
        let b = node_box(&cx, &busy);
        assert_eq!(
            (b.actions, b.action_rows, b.more, b.actions_y, b.h),
            (6, 4, true, 34.0, 34.0 + 100.0 + 6.0)
        );

        // a trigger runs no actions; a collapsed node is its header
        let mut t = Node::new("go", "trigger.start");
        t.on_enter.push(crate::asset::Action { kind: "x".into(), params: gz_luadata::Value::Nil });
        assert_eq!(node_box(&cx, &t).actions, 0);
        busy.collapsed = true;
        assert_eq!(node_box(&cx, &busy).h, HEADER);

        // flowing right, the outputs stack down the right edge: the box makes room for them
        let right = BoxCx { flow: Flow::Right, ..cx };
        let mut two = Node::new("two", "dialog.topic");
        two.collapsed = true;
        assert_eq!(node_box(&right, &two).h, HEADER + PIN_PITCH + PAD);
        assert_eq!(output_pin(Flow::Right, [0.0, 0.0], [200.0, 60.0], 1, 2), [200.0, 14.0 + 18.0]);
        assert_eq!(input_pin(Flow::Down, [0.0, 0.0], [200.0, 60.0]), [100.0, 0.0]);
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
        let l = lines(&cat, &n, false);
        assert_eq!(l.len(), 3);
        assert_eq!(l.get(1).map(|x| x.key.as_str()), Some("Not b"));
        // four lines, three shown: two of them behind the last
        assert_eq!(l.get(2).map(|x| x.value.as_str()), Some("+2 more"));
        assert_eq!(cond_key("Has money", true), "Not has money");
        assert_eq!(cond_key("NPC alive", true), "Not NPC alive");
    }

    #[test]
    fn the_default_pin() {
        let p = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(default_pin(&p(&["done", "timeout"])), Some("done"));
        assert_eq!(default_pin(&p(&["timeout", "else"])), Some("else"));
        assert_eq!(default_pin(&p(&["started", "finished", "aborted"])), Some("finished"));
        assert!(is_failure_pin("timeout") && !is_failure_pin("done"));
    }
}
