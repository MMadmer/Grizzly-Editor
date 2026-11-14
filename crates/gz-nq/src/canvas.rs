//! `canvas = { ... }`: what a graph's file keeps for the editor alone (docs/nq/NQ_FORMAT.md,
//! "The canvas"). The game ignores the whole table.
//!
//! - `flow`: `"right"` (inputs on a node's left edge, outputs on its right) or `"down"`. A file
//!   without it that has node positions was laid out top to bottom; one without positions takes
//!   the author's preference.
//! - `wires`: the wire options of this graph; every key left out is the preference's.
//! - `frames`: comment frames - a title, a tone (a `graph.family` index) and a rectangle.
//! - `routes`: the waypoints of a wire, keyed by `"<from>.<pin>><to>"`.
//! - `bookmarks`: named places, each a node.

use gz_luadata::{Table, Value};
use smol_str::SmolStr;

/// Which way a graph flows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Flow {
    /// Left to right: inputs on a node's left edge, outputs on its right.
    #[default]
    Right,
    /// Top to bottom: the input on the top edge, outputs on the bottom.
    Down,
}

impl Flow {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "right" => Some(Self::Right),
            "down" => Some(Self::Down),
            _ => None,
        }
    }

    /// The flow's own axis is x (`Right`) or y (`Down`).
    #[must_use]
    pub fn is_right(self) -> bool {
        self == Self::Right
    }
}

/// How a wire is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WireStyle {
    /// A bezier whose tangent grows with both deltas.
    #[default]
    Curved,
    /// Straight between short stubs out of the pins.
    Straight,
    /// Right angles with round corners, sibling wires on lanes of their own.
    Circuit,
    /// 45 degree diagonals between straight stubs.
    Angled,
    /// Angled, the wires of one output sharing a trunk until their targets part.
    Subway,
}

impl WireStyle {
    pub const ALL: [WireStyle; 5] = [Self::Curved, Self::Straight, Self::Circuit, Self::Angled, Self::Subway];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Curved => "curved",
            Self::Straight => "straight",
            Self::Circuit => "circuit",
            Self::Angled => "angled",
            Self::Subway => "subway",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Curved => "Curved",
            Self::Straight => "Straight",
            Self::Circuit => "Circuit",
            Self::Angled => "Angled",
            Self::Subway => "Subway",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == s)
    }
}

/// Where the 45 degree part of an angled wire sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Diagonal {
    Start,
    #[default]
    Middle,
    End,
}

impl Diagonal {
    pub const ALL: [Diagonal; 3] = [Self::Start, Self::Middle, Self::End];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.key() == s)
    }
}

/// The wire options of one graph (`canvas.wires`); `None` is "as in Preferences".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireOpts {
    pub style: Option<WireStyle>,
    pub diagonal: Option<Diagonal>,
    /// Px at 100%, 1-4.
    pub thickness: Option<f32>,
    pub by_pin: Option<bool>,
    pub dim: Option<bool>,
    pub bubbles: Option<bool>,
    pub arrows: Option<bool>,
}

impl WireOpts {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// These options over `base`: what is set here wins.
    #[must_use]
    pub fn over(&self, base: &WireSettings) -> WireSettings {
        WireSettings {
            style: self.style.unwrap_or(base.style),
            diagonal: self.diagonal.unwrap_or(base.diagonal),
            thickness: self.thickness.unwrap_or(base.thickness).clamp(1.0, 4.0),
            by_pin: self.by_pin.unwrap_or(base.by_pin),
            dim: self.dim.unwrap_or(base.dim),
            bubbles: self.bubbles.unwrap_or(base.bubbles),
            arrows: self.arrows.unwrap_or(base.arrows),
        }
    }
}

/// The wire options in force: the preferences with a graph's own over them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireSettings {
    pub style: WireStyle,
    pub diagonal: Diagonal,
    pub thickness: f32,
    pub by_pin: bool,
    pub dim: bool,
    pub bubbles: bool,
    pub arrows: bool,
}

impl Default for WireSettings {
    fn default() -> Self {
        Self {
            style: WireStyle::Curved,
            diagonal: Diagonal::Middle,
            thickness: 2.0,
            by_pin: true,
            dim: false,
            bubbles: false,
            arrows: true,
        }
    }
}

/// A comment frame: geometry, not membership - moving it moves the nodes inside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub title: String,
    /// A `graph.family` index, 0-9.
    pub tone: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Frame {
    #[must_use]
    pub fn rect(&self) -> [f32; 4] {
        [self.x, self.y, self.x + self.w, self.y + self.h]
    }

    /// A box lies wholly inside the frame.
    #[must_use]
    pub fn holds(&self, r: [f32; 4]) -> bool {
        r[0] >= self.x && r[1] >= self.y && r[2] <= self.x + self.w && r[3] <= self.y + self.h
    }
}

/// A named place: a node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub node: SmolStr,
}

/// What of the table the file wrote, so a save writes it back even when it empties.
const HAS_CANVAS: u8 = 1;
const HAS_WIRES: u8 = 2;
const HAS_FRAMES: u8 = 4;
const HAS_ROUTES: u8 = 8;
const HAS_BOOKMARKS: u8 = 16;

/// The graph's `canvas` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    /// As the file says it.
    pub flow: Option<Flow>,
    /// What a graph without `flow` flows as: `Down` when the file has positions, else the
    /// author's preference.
    pub implied: Flow,
    pub wires: WireOpts,
    pub frames: Vec<Frame>,
    /// `"<from>.<pin>><to>"` and the waypoints, in file order.
    pub routes: Vec<(String, Vec<[f32; 2]>)>,
    pub bookmarks: Vec<Bookmark>,
    present: u8,
}

/// The key of a wire's route.
#[must_use]
pub fn route_key(from: &str, pin: &str, to: &str) -> String {
    format!("{from}.{pin}>{to}")
}

/// `(from, pin, to)` of a route key.
#[must_use]
pub fn split_route_key(key: &str) -> Option<(&str, &str, &str)> {
    let (src, to) = key.split_once('>')?;
    let (from, pin) = src.split_once('.')?;
    Some((from, pin, to))
}

fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) if n.is_finite() => Some(*n as f32),
        _ => None,
    }
}

fn point(v: &Value) -> Option<[f32; 2]> {
    let a = v.arr();
    if a.len() >= 2 {
        return Some([num(a.first()?)?, num(a.get(1)?)?]);
    }
    if v.has("x") && v.has("y") {
        return Some([num(v.get("x")?)?, num(v.get("y")?)?]);
    }
    None
}

impl Canvas {
    /// The flow in force.
    #[must_use]
    pub fn flow(&self) -> Flow {
        self.flow.unwrap_or(self.implied)
    }

    /// The route of a wire.
    #[must_use]
    pub fn route(&self, from: &str, pin: &str, to: &str) -> Option<&[[f32; 2]]> {
        let key = route_key(from, pin, to);
        self.routes.iter().find(|(k, _)| *k == key).map(|(_, r)| r.as_slice())
    }

    /// Sets (or with no points, drops) the route of a wire.
    pub fn set_route(&mut self, from: &str, pin: &str, to: &str, points: Vec<[f32; 2]>) {
        let key = route_key(from, pin, to);
        match self.routes.iter().position(|(k, _)| *k == key) {
            Some(i) if points.is_empty() => {
                self.routes.remove(i);
            }
            Some(i) => {
                if let Some(r) = self.routes.get_mut(i) {
                    r.1 = points;
                }
            }
            None if points.is_empty() => {}
            None => self.routes.push((key, points)),
        }
    }

    /// A node renamed: the route keys and the bookmarks follow it.
    pub fn rename_node(&mut self, id: &str, new_id: &str) {
        for (key, _) in &mut self.routes {
            if let Some((from, pin, to)) = split_route_key(key) {
                let from = if from == id { new_id } else { from };
                let to = if to == id { new_id } else { to };
                *key = route_key(from, pin, to);
            }
        }
        for b in &mut self.bookmarks {
            if b.node == id {
                b.node = SmolStr::new(new_id);
            }
        }
    }

    /// Drops what names a node that went: its routes and its bookmarks.
    pub fn forget_node(&mut self, id: &str) {
        self.routes.retain(|(k, _)| split_route_key(k).is_none_or(|(f, _, t)| f != id && t != id));
        self.bookmarks.retain(|b| b.node != id);
    }

    /// Keeps only the routes of wires that exist.
    pub fn prune_routes(&mut self, exists: impl Fn(&str, &str, &str) -> bool) {
        self.routes.retain(|(k, _)| split_route_key(k).is_some_and(|(f, p, t)| exists(f, p, t)));
    }

    /// The canvas of a file's `canvas` value; what does not read is reported and left out.
    #[must_use]
    pub fn from_value(v: Option<&Value>, errs: &mut Vec<String>) -> Self {
        let mut c = Canvas::default();
        let Some(v) = v else { return c };
        if !v.is_table() {
            errs.push("canvas must be a table".to_owned());
            return c;
        }
        c.present |= HAS_CANVAS;
        if let Some(f) = v.get("flow") {
            match f.as_str().and_then(Flow::parse) {
                Some(flow) => c.flow = Some(flow),
                None => errs.push("canvas.flow must be \"right\" or \"down\"".to_owned()),
            }
        }
        if let Some(w) = v.get("wires") {
            if w.is_table() {
                c.present |= HAS_WIRES;
                c.wires = wires_from(w, errs);
            } else {
                errs.push("canvas.wires must be a table".to_owned());
            }
        }
        if let Some(fs) = v.get("frames") {
            c.present |= HAS_FRAMES;
            for (i, f) in fs.arr().iter().enumerate() {
                let (Some(x), Some(y), Some(w), Some(h)) = (
                    f.get("x").and_then(num),
                    f.get("y").and_then(num),
                    f.get("w").and_then(num),
                    f.get("h").and_then(num),
                ) else {
                    errs.push(format!("canvas.frames #{}: a frame needs x, y, w and h", i + 1));
                    continue;
                };
                c.frames.push(Frame {
                    title: f.get_string("title", ""),
                    tone: f.get_number("tone", 0.0).clamp(0.0, 9.0) as u8,
                    x,
                    y,
                    w: w.max(32.0),
                    h: h.max(32.0),
                });
            }
        }
        if let Some(rs) = v.get("routes") {
            c.present |= HAS_ROUTES;
            for (key, pts) in rs.fields() {
                if split_route_key(key).is_none() {
                    errs.push(format!("canvas.routes: '{key}' is no wire (from.pin>to)"));
                    continue;
                }
                let points: Vec<[f32; 2]> = pts.arr().iter().filter_map(point).collect();
                if !points.is_empty() {
                    c.routes.push((key.to_string(), points));
                }
            }
        }
        if let Some(bs) = v.get("bookmarks") {
            c.present |= HAS_BOOKMARKS;
            for b in bs.arr() {
                let node = b.get_string("node", "");
                if node.is_empty() {
                    errs.push("canvas.bookmarks: a bookmark needs its node".to_owned());
                    continue;
                }
                c.bookmarks.push(Bookmark { name: b.get_string("name", &node), node: SmolStr::new(node) });
            }
        }
        c
    }

    /// The canvas as a save compares it: `None` when the file has no canvas and nothing needs
    /// one. `flow` is written for a graph whose flow came from the preference once its nodes are
    /// placed (`placed`): without it the file would read as flowing down next time.
    #[must_use]
    pub fn to_value(&self, placed: bool) -> Option<Value> {
        let mut t = Table::new();
        let flow = self.flow.or_else(|| (placed && self.implied != Flow::Down).then_some(self.implied));
        if let Some(f) = flow {
            t.put("flow", f.key());
        }
        if self.present & HAS_WIRES != 0 || !self.wires.is_empty() {
            t.put("wires", wires_to(&self.wires));
        }
        if self.present & HAS_FRAMES != 0 || !self.frames.is_empty() {
            let frames: Vec<Value> = self
                .frames
                .iter()
                .map(|f| {
                    let mut ft = Table::new();
                    ft.put("title", f.title.as_str());
                    ft.put("tone", f64::from(f.tone));
                    ft.put("x", f64::from(f.x));
                    ft.put("y", f64::from(f.y));
                    ft.put("w", f64::from(f.w));
                    ft.put("h", f64::from(f.h));
                    Value::Table(ft)
                })
                .collect();
            t.put("frames", Value::from(frames));
        }
        if self.present & HAS_ROUTES != 0 || !self.routes.is_empty() {
            let mut rt = Table::new();
            for (k, pts) in &self.routes {
                let list: Vec<Value> = pts
                    .iter()
                    .map(|p| Value::from(vec![Value::from(f64::from(p[0])), Value::from(f64::from(p[1]))]))
                    .collect();
                rt.put(k, Value::from(list));
            }
            t.put("routes", rt);
        }
        if self.present & HAS_BOOKMARKS != 0 || !self.bookmarks.is_empty() {
            let list: Vec<Value> = self
                .bookmarks
                .iter()
                .map(|b| {
                    let mut bt = Table::new();
                    bt.put("name", b.name.as_str());
                    bt.put("node", b.node.as_str());
                    Value::Table(bt)
                })
                .collect();
            t.put("bookmarks", Value::from(list));
        }
        if t.fields.is_empty() && self.present & HAS_CANVAS == 0 {
            return None;
        }
        Some(Value::Table(t))
    }
}

fn wires_from(w: &Value, errs: &mut Vec<String>) -> WireOpts {
    let mut o = WireOpts::default();
    if let Some(s) = w.get("style") {
        match s.as_str().and_then(WireStyle::parse) {
            Some(style) => o.style = Some(style),
            None => {
                errs.push("canvas.wires.style must be curved, straight, circuit, angled or subway".to_owned())
            }
        }
    }
    if let Some(d) = w.get("diagonal") {
        match d.as_str().and_then(Diagonal::parse) {
            Some(diag) => o.diagonal = Some(diag),
            None => errs.push("canvas.wires.diagonal must be start, middle or end".to_owned()),
        }
    }
    if let Some(t) = w.get("thickness").and_then(num) {
        o.thickness = Some(t.clamp(1.0, 4.0));
    }
    let flag = |key: &str| w.get(key).and_then(Value::as_bool);
    o.by_pin = flag("by_pin");
    o.dim = flag("dim");
    o.bubbles = flag("bubbles");
    o.arrows = flag("arrows");
    o
}

fn wires_to(o: &WireOpts) -> Table {
    let mut t = Table::new();
    if let Some(s) = o.style {
        t.put("style", s.key());
    }
    if let Some(d) = o.diagonal {
        t.put("diagonal", d.key());
    }
    if let Some(th) = o.thickness {
        t.put("thickness", f64::from(th));
    }
    for (key, v) in [("by_pin", o.by_pin), ("dim", o.dim), ("bubbles", o.bubbles), ("arrows", o.arrows)] {
        if let Some(b) = v {
            t.put(key, b);
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_keys_follow_a_rename() {
        let mut c = Canvas::default();
        c.set_route("wait", "done", "ambush", vec![[400.0, 120.0], [400.0, 260.0]]);
        c.bookmarks.push(Bookmark { name: "Finale".into(), node: "ambush".into() });
        c.rename_node("ambush", "trap");
        assert_eq!(c.route("wait", "done", "trap").map(<[[f32; 2]]>::len), Some(2));
        assert!(c.route("wait", "done", "ambush").is_none());
        assert_eq!(c.bookmarks[0].node, "trap");
        c.forget_node("wait");
        assert!(c.routes.is_empty());
        assert_eq!(split_route_key("a.b>c"), Some(("a", "b", "c")));
    }

    #[test]
    fn flow_is_written_once_the_nodes_are_placed() {
        let c = Canvas { implied: Flow::Right, ..Canvas::default() };
        assert!(c.to_value(false).is_none());
        let v = c.to_value(true).unwrap_or_default();
        assert_eq!(v.get_string("flow", ""), "right");
        let down = Canvas { implied: Flow::Down, ..Canvas::default() };
        assert!(down.to_value(true).is_none());
    }
}
