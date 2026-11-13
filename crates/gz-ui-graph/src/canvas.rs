//! The graph canvas (`NqCanvas`): an infinite world painted by hand, right or middle drag pans,
//! the wheel zooms at the cursor over ten fixed levels, the flow runs top-down (input on the top
//! edge, one output pin per catalog pin on the bottom edge). A node is drawn from the scene's box
//! (`gz_nq::layout`), only when the view touches its grid cell. Every edit is a `gz_nq` document
//! command the tab sends; what the canvas keeps is view state: the gesture in progress, what is
//! under the cursor, the last status.

use std::sync::Arc;

use egui::epaint::{CubicBezierShape, Mesh, PathShape};
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use gz_luadata::Value;
use gz_nq::asset::{self, Graph, Node};
use gz_nq::catalog::{USE_EXTRA, USE_MAIN, USE_TRIGGER};
use gz_nq::doc::{self, CanvasOp, DocOp, ViewOp, ZOOM, ZOOM_DEFAULT, zoom_of};
use gz_nq::layout::{self, ACTION_ROW, GRID, HEADER, LINE, PAD, PIN_ROW};
use gz_nq::{Catalog, text};
use gz_ui::menu::{MenuItem, menu_item, menu_popup};
use gz_ui::{Font, Icon, Palette, icons, palette};
use smol_str::SmolStr;

use crate::derived::DocData;
use crate::local;
use crate::look::{self, Family};
use crate::scene::{self, MINIMAP_H, MINIMAP_W, Scene, SceneLink, SceneNode};

const TITLE_PT: f32 = 16.0;
const LINE_PT: f32 = 15.0;
const SMALL_PT: f32 = 14.0;
const LOD_FULL: f32 = 10.0;
const LOD_TITLES: f32 = 7.0;
const PIN_R: f32 = 4.5;
const INSET: f32 = 8.0;
const PIN_GRAB_PAD: f32 = 8.0;
const PIN_GRAB_MIN: f32 = 12.0;
const PIN_GRAB_MAX: f32 = 22.0;
const PIN_GRAB_INSIDE: f32 = 0.7;
const LINK_GRAB: f32 = 12.0;
const STATUS_SECONDS: f64 = 2.5;
/// A view at rest this long after a wheel step reaches the document (and its history).
const WHEEL_SETTLE: f64 = 0.35;

/// The view the tab mirrors: what the canvas changes at once, the document learns after.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewMirror {
    pub center: [f32; 2],
    pub zoom: i32,
    pub selection: Vec<SmolStr>,
    pub slot: String,
}

impl Default for ViewMirror {
    fn default() -> Self {
        Self { center: [0.0, 0.0], zoom: ZOOM_DEFAULT, selection: Vec::new(), slot: String::new() }
    }
}

impl ViewMirror {
    #[must_use]
    pub fn of(v: &doc::View) -> Self {
        Self { center: v.center, zoom: v.zoom, selection: v.selection.clone(), slot: v.slot.clone() }
    }

    #[must_use]
    pub fn is_selected(&self, id: &str) -> bool {
        self.selection.iter().any(|s| s == id)
    }

    pub fn select_only(&mut self, id: &str) {
        self.selection.clear();
        if !id.is_empty() {
            self.selection.push(SmolStr::new(id));
        }
        self.slot.clear();
    }

    fn toggle(&mut self, id: &str) {
        match self.selection.iter().position(|s| s == id) {
            Some(i) => {
                self.selection.remove(i);
            }
            None => self.selection.push(SmolStr::new(id)),
        }
    }
}

/// What the canvas asks of the tab.
// a few per frame at most, consumed at once: the size of a document op costs nothing
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A command of the document (content or view).
    Doc(DocOp),
    /// Centre a node at 1:1 or closer and select it (one history step).
    FrameNode(String),
    Undo,
    Redo,
    /// Opens an action of the selected node (after a `FrameNode` that selected it).
    Slot(String),
}

/// An action row under the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ActHit {
    node: usize,
    row: usize,
    /// The `+N more` row.
    more: bool,
}

#[derive(Clone, Debug)]
struct Drag {
    /// node index, id, start position
    nodes: Vec<(usize, SmolStr, [f32; 2])>,
    start: Pos2,
    moved: bool,
    delta: [f32; 2],
}

/// Positions committed by a drag, drawn until the scene that holds them arrives.
#[derive(Clone, Debug)]
struct Settle {
    from_edit: u64,
    at: Vec<(usize, [f32; 2])>,
}

#[derive(Clone, Debug)]
struct ActDrag {
    node: SmolStr,
    exit: bool,
    index: usize,
    start: Pos2,
    dragging: bool,
}

/// The popup of the canvas that is up.
#[derive(Clone, Debug, PartialEq)]
enum Menu {
    Node(SmolStr),
    Action {
        node: SmolStr,
        exit: bool,
        index: usize,
    },
    Link {
        from: SmolStr,
        pin: SmolStr,
        to: SmolStr,
    },
    /// Add a node at a world point; `link` wires it from a pin.
    AddNode {
        at: [f32; 2],
        link: Option<(SmolStr, SmolStr)>,
    },
    AddAction {
        node: SmolStr,
        exit: bool,
    },
}

/// The canvas of one tab.
pub struct Canvas {
    id: Id,
    pub origin: Pos2,
    pub size: Vec2,
    /// Frame number the canvas was last drawn in.
    pub drawn_frame: u64,
    pub hovered: bool,
    /// Keys go to it: the last click in the tab was on it.
    pub focused: bool,
    pub minimap_shown: bool,
    minimap_hovered: bool,
    minimap_panning: bool,
    minimap_cache: Option<(u64, Pos2, Arc<Mesh>)>,
    pub beyond: [usize; 4],
    beyond_near: [Option<u32>; 4],
    beyond_hot: Option<usize>,
    pan: Option<(Pos2, bool)>,
    middle_pan: bool,
    drag: Option<Drag>,
    settle: Option<Settle>,
    marquee: Option<Pos2>,
    linking: Option<(SmolStr, SmolStr)>,
    act_drag: Option<ActDrag>,
    hot_node: Option<usize>,
    /// The pin of `hot_node` under the cursor; `None` with a node is its input.
    hot_pin: Option<usize>,
    hot_link: Option<usize>,
    hot_action: Option<ActHit>,
    status: String,
    status_at: f64,
    tip: String,
    menu: Option<Menu>,
    menu_pos: Pos2,
    next_menu: Option<(Menu, Pos2)>,
    filter: String,
    filter_sel: usize,
    filter_focus: bool,
    want_frame_all: bool,
    want_frame_sel: bool,
    /// A zoom asked for explicitly: applied after a pending frame picked its own.
    pub want_zoom: Option<i32>,
    /// F2 or a rename from the menu: the Details panel focuses the id.
    pub want_rename: bool,
    /// A double click on an action row: the Details panel focuses its kind.
    pub want_focus_action: bool,
    /// The view moved by a gesture that has not reached the document yet.
    pub view_moving: bool,
    last_wheel: f64,
    stamp: Vec<u32>,
    serial: u32,
    scratch: Vec<u32>,
    visible: Vec<usize>,
}

/// What a frame of the canvas reads and where it writes.
pub struct CanvasCx<'a> {
    pub scene: Option<&'a Scene>,
    pub graph: &'a Graph,
    pub catalog: &'a Catalog,
    pub data: &'a DocData,
    pub view: &'a mut ViewMirror,
    pub bookmarks: &'a [SmolStr],
    pub minimap: bool,
    pub ops: &'a mut Vec<Op>,
    /// The document's content serial now (a drag's commit waits for a scene past it).
    pub edit: u64,
    /// The status the document's last canvas command left, when it is new.
    pub doc_status: Option<String>,
    /// What the tab draws over the canvas (the Keyboard Shortcuts card, the rail's Graph panel), as the
    /// last frame laid it out: the pointer over one of them is not over the canvas - a click, the wheel
    /// and a hover there are that panel's.
    pub covered: &'a [Rect],
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn seg_dist2(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let l2 = ab.length_sq();
    let t = if l2 > 0.0 { (ap.dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length_sq()
}

fn bezier(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    pos2(a * p0.x + b * p1.x + c * p2.x + d * p3.x, a * p0.y + b * p1.y + c * p2.y + d * p3.y)
}

fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

fn round(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// One line of canvas text cut to `avail`, painted with its left edge at `at.x` and its middle
/// at `at.y`; the width drawn (`PaintFit`). Canvas text is clipped by its node on purpose, so it
/// is not reported to the audit.
fn paint_fit(painter: &Painter, font: &FontId, at: Pos2, color: Color32, text_: &str, avail: f32) -> f32 {
    if text_.is_empty() || avail <= font.size * 0.5 {
        return 0.0;
    }
    let (shown, _) = gz_ui::text::ellipsize(painter.ctx(), font, text_, avail, gz_ui::Clip::End);
    painter.text(at, Align2::LEFT_CENTER, &shown, font.clone(), color).width()
}

/// A slot name the document uses.
fn slot_name(exit: bool) -> &'static str {
    if exit { "exit" } else { "enter" }
}

fn op_table(fields: &[(&str, Value)]) -> Value {
    let mut t = Value::table();
    for (k, v) in fields {
        t.set(k, v.clone());
    }
    t
}

impl Canvas {
    #[must_use]
    pub fn new(salt: &str, frame_on_open: bool) -> Self {
        Self {
            id: Id::unique(("gz-ui-graph.canvas", salt)),
            origin: Pos2::ZERO,
            size: Vec2::ZERO,
            drawn_frame: 0,
            hovered: false,
            focused: false,
            minimap_shown: false,
            minimap_hovered: false,
            minimap_panning: false,
            minimap_cache: None,
            beyond: [0; 4],
            beyond_near: [None; 4],
            beyond_hot: None,
            pan: None,
            middle_pan: false,
            drag: None,
            settle: None,
            marquee: None,
            linking: None,
            act_drag: None,
            hot_node: None,
            hot_pin: None,
            hot_link: None,
            hot_action: None,
            status: String::new(),
            status_at: 0.0,
            tip: String::new(),
            menu: None,
            menu_pos: Pos2::ZERO,
            next_menu: None,
            filter: String::new(),
            filter_sel: 0,
            filter_focus: false,
            want_frame_all: frame_on_open,
            want_frame_sel: false,
            want_zoom: None,
            want_rename: false,
            want_focus_action: false,
            view_moving: false,
            last_wheel: f64::MIN,
            stamp: Vec::new(),
            serial: 0,
            scratch: Vec::new(),
            visible: Vec::new(),
        }
    }

    // ---- the view ---------------------------------------------------------------------------

    /// Points per world unit: the zoom level's factor (egui's unit is the point).
    #[must_use]
    pub fn scale(view: &ViewMirror) -> f32 {
        zoom_of(view.zoom)
    }

    /// The px size node titles are drawn at on this level (`TitlePx`).
    #[must_use]
    pub fn title_px(view: &ViewMirror, ppp: f32) -> f32 {
        round(TITLE_PT * zoom_of(view.zoom) * ppp)
    }

    fn lod(view: &ViewMirror, ppp: f32) -> u8 {
        let px = Self::title_px(view, ppp);
        if px >= LOD_FULL {
            2
        } else if px >= LOD_TITLES {
            1
        } else {
            0
        }
    }

    /// A size of canvas type at this level: the line box rounded to whole px, as an egui font.
    fn font(face: Font, line_pt: f32, view: &ViewMirror, ppp: f32) -> FontId {
        let px = round(line_pt * zoom_of(view.zoom) * ppp).max(1.0);
        face.at(px / ppp * 0.75)
    }

    #[must_use]
    pub fn to_screen(&self, view: &ViewMirror, w: [f32; 2]) -> Pos2 {
        let s = Self::scale(view);
        pos2(
            self.origin.x + self.size.x * 0.5 + (w[0] - view.center[0]) * s,
            self.origin.y + self.size.y * 0.5 + (w[1] - view.center[1]) * s,
        )
    }

    #[must_use]
    pub fn to_world(&self, view: &ViewMirror, p: Pos2) -> [f32; 2] {
        let s = Self::scale(view);
        [
            view.center[0] + (p.x - self.origin.x - self.size.x * 0.5) / s,
            view.center[1] + (p.y - self.origin.y - self.size.y * 0.5) / s,
        ]
    }

    /// The world rectangle the view shows, `None` before the canvas was measured.
    #[must_use]
    pub fn viewport(&self, view: &ViewMirror) -> Option<[f32; 4]> {
        if !self.measured() {
            return None;
        }
        let s = Self::scale(view);
        let (hw, hh) = (self.size.x / (2.0 * s), self.size.y / (2.0 * s));
        Some([view.center[0] - hw, view.center[1] - hh, view.center[0] + hw, view.center[1] + hh])
    }

    #[must_use]
    pub fn measured(&self) -> bool {
        self.size.x > 32.0 && self.size.y > 32.0
    }

    /// Frame All waits for a drawn canvas and its scene.
    pub fn request_frame_all(&mut self) {
        self.want_frame_all = true;
    }

    pub fn request_frame_selection(&mut self) {
        self.want_frame_sel = true;
    }

    /// A frame asked for and not done yet.
    #[must_use]
    pub fn frame_pending(&self) -> bool {
        self.want_frame_all || self.want_frame_sel
    }

    /// An explicit view (quest_view's zoom or centre) wins over a pending frame.
    pub fn cancel_framing(&mut self) {
        self.want_frame_all = false;
        self.want_frame_sel = false;
    }

    pub fn set_status(&mut self, ctx: &egui::Context, text_: &str) {
        text_.clone_into(&mut self.status);
        self.status_at = ctx.input(|i| i.time);
    }

    /// One zoom step, around a screen point when given (`ZoomBy`).
    pub fn zoom_by(&self, view: &mut ViewMirror, delta: i32, anchor: Option<Pos2>) {
        let z = (view.zoom + delta).clamp(0, ZOOM.len() as i32 - 1);
        if z == view.zoom {
            return;
        }
        match anchor {
            Some(a) => {
                let w = self.to_world(view, a);
                view.zoom = z;
                let p = self.to_screen(view, w);
                let s = Self::scale(view);
                view.center = [
                    layout::sane(view.center[0] + (p.x - a.x) / s),
                    layout::sane(view.center[1] + (p.y - a.y) / s),
                ];
            }
            None => view.zoom = z,
        }
    }

    fn frame_all_now(&mut self, ctx: &egui::Context, sc: &Scene, view: &mut ViewMirror) {
        let f = scene::frame_all(sc, [self.size.x, self.size.y]);
        view.zoom = f.zoom;
        view.center = f.center;
        if let Some(s) = f.status {
            self.set_status(ctx, &s);
        }
    }

    fn frame_selection_now(&mut self, ctx: &egui::Context, sc: &Scene, view: &mut ViewMirror) {
        let mut r: Option<[f32; 4]> = None;
        for id in &view.selection {
            let Some(n) = sc.node_of(id).and_then(|i| sc.nodes.get(i)) else { continue };
            let b = n.rect();
            r = Some(r.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]));
        }
        match r {
            None => self.frame_all_now(ctx, sc, view),
            Some(r) => {
                let (z, c) = scene::frame_rect(r, [self.size.x, self.size.y]);
                view.zoom = z;
                view.center = c;
            }
        }
    }

    /// Brings the nearest node beyond an edge into view (`RevealBeyond`).
    fn reveal_beyond(&self, sc: &Scene, view: &mut ViewMirror, side: usize) {
        let Some(n) = self.beyond_near.get(side).copied().flatten().and_then(|i| sc.nodes.get(i as usize))
        else {
            return;
        };
        let Some(v) = self.viewport(view) else { return };
        let m = scene::FRAME_MARGIN;
        let (mut dx, mut dy) = (0.0, 0.0);
        match side {
            0 => dy = n.pos[1] - m - v[1],
            1 => dy = (n.pos[1] + n.size[1] + m - v[3]).min(n.pos[1] - m - v[1]),
            2 => dx = n.pos[0] - m - v[0],
            _ => dx = (n.pos[0] + n.size[0] + m - v[2]).min(n.pos[0] - m - v[0]),
        }
        view.center = [layout::sane(view.center[0] + dx), layout::sane(view.center[1] + dy)];
    }

    // ---- geometry under a drag ----------------------------------------------------------------

    /// Where a node is drawn now: the scene's place, moved by a drag or a commit in flight.
    fn pos_of(&self, sc: &Scene, i: usize) -> [f32; 2] {
        let base = sc.nodes.get(i).map_or([0.0, 0.0], |n| n.pos);
        if let Some(d) = &self.drag
            && d.moved
            && let Some((_, _, start)) = d.nodes.iter().find(|(j, _, _)| *j == i)
        {
            return add(*start, d.delta);
        }
        if let Some(s) = &self.settle
            && let Some((_, p)) = s.at.iter().find(|(j, _)| *j == i)
        {
            return *p;
        }
        base
    }

    fn moving(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moved) || self.settle.is_some()
    }

    fn rect_of(&self, sc: &Scene, view: &ViewMirror, i: usize) -> Rect {
        let n = sc.nodes.get(i);
        let p = self.pos_of(sc, i);
        let size = n.map_or([0.0, 0.0], |n| n.size);
        Rect::from_min_max(self.to_screen(view, p), self.to_screen(view, add(p, size)))
    }

    fn output_pin(&self, sc: &Scene, view: &ViewMirror, i: usize, pin: usize) -> Pos2 {
        let Some(n) = sc.nodes.get(i) else { return Pos2::ZERO };
        self.to_screen(view, layout::output_pin(self.pos_of(sc, i), n.size, pin, n.pins.len()))
    }

    fn input_pin(&self, sc: &Scene, view: &ViewMirror, i: usize) -> Pos2 {
        let Some(n) = sc.nodes.get(i) else { return Pos2::ZERO };
        self.to_screen(view, layout::input_pin(self.pos_of(sc, i), n.size))
    }

    /// The candidate nodes around a screen point, deduped, highest index first.
    fn candidates(&mut self, sc: &Scene, view: &ViewMirror, p: Pos2, r: f32) -> Vec<usize> {
        let s = Self::scale(view);
        let w = self.to_world(view, p);
        let rw = r / s + 1.0;
        self.scratch.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        sc.nodes_in([w[0] - rw, w[1] - rw, w[0] + rw, w[1] + rw], &mut scratch);
        let mut out = self.dedupe(sc.nodes.len(), &scratch);
        self.scratch = scratch;
        if self.moving() {
            // a dragged node is not where its cell says
            if let Some(d) = &self.drag {
                out.extend(d.nodes.iter().map(|(i, _, _)| *i));
            }
            if let Some(st) = &self.settle {
                out.extend(st.at.iter().map(|(i, _)| *i));
            }
            out.sort_unstable();
            out.dedup();
        }
        out.reverse();
        out
    }

    fn dedupe(&mut self, n: usize, ids: &[u32]) -> Vec<usize> {
        if self.stamp.len() < n {
            self.stamp.resize(n, 0);
        }
        self.serial = self.serial.wrapping_add(1).max(1);
        let mut out = Vec::with_capacity(ids.len());
        for &i in ids {
            if let Some(s) = self.stamp.get_mut(i as usize)
                && *s != self.serial
            {
                *s = self.serial;
                out.push(i as usize);
            }
        }
        out.sort_unstable();
        out
    }

    fn pin_grab(view: &ViewMirror, ppp: f32) -> f32 {
        let drawn = (2.0 / ppp).max(PIN_R * Self::scale(view));
        (drawn + PIN_GRAB_PAD).clamp(PIN_GRAB_MIN, PIN_GRAB_MAX)
    }

    /// `inward` points from the pin into its node (-1 for an output on the bottom edge, +1 for
    /// the input on the top one): that half of the zone stays narrow (`NearPin`).
    fn near_pin(p: Pos2, c: Pos2, inward: f32, grab: f32) -> Option<f32> {
        let mut d = p - c;
        if d.y * inward > 0.0 {
            d.y /= PIN_GRAB_INSIDE;
        }
        let d2 = d.length_sq();
        (d2 <= grab * grab).then_some(d2)
    }

    fn hit_node(&mut self, sc: &Scene, view: &ViewMirror, p: Pos2) -> Option<usize> {
        let cands = self.candidates(sc, view, p, 1.0);
        cands.into_iter().find(|&i| self.rect_of(sc, view, i).contains(p))
    }

    /// The nearest node within the grab radius of its box, `except` skipped (`HitNodeNear`).
    fn hit_node_near(
        &mut self,
        sc: &Scene,
        view: &ViewMirror,
        p: Pos2,
        except: Option<usize>,
        grab: f32,
    ) -> Option<usize> {
        let mut best = f32::MAX;
        let mut hit = None;
        for i in self.candidates(sc, view, p, grab) {
            if Some(i) == except {
                continue;
            }
            let r = self.rect_of(sc, view, i);
            let dx = if p.x < r.min.x { r.min.x - p.x } else { (p.x - r.max.x).max(0.0) };
            let dy = if p.y < r.min.y { r.min.y - p.y } else { (p.y - r.max.y).max(0.0) };
            let d = dx * dx + dy * dy;
            if d <= grab * grab && d < best {
                best = d;
                hit = Some(i);
            }
        }
        hit
    }

    fn hit_pin(
        &self,
        sc: &Scene,
        view: &ViewMirror,
        node: usize,
        p: Pos2,
        grab: f32,
    ) -> Option<(usize, f32)> {
        let n = sc.nodes.get(node)?;
        let mut best: Option<(usize, f32)> = None;
        for k in 0..n.pins.len() {
            if let Some(d2) = Self::near_pin(p, self.output_pin(sc, view, node, k), -1.0, grab)
                && best.is_none_or(|(_, b)| d2 < b)
            {
                best = Some((k, d2));
            }
        }
        best
    }

    fn hit_any_pin(&mut self, sc: &Scene, view: &ViewMirror, p: Pos2, grab: f32) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in self.candidates(sc, view, p, grab) {
            if let Some((k, d2)) = self.hit_pin(sc, view, i, p, grab)
                && best.is_none_or(|(_, _, b)| d2 < b)
            {
                best = Some((i, k, d2));
            }
        }
        best.map(|(i, k, _)| (i, k))
    }

    fn hit_action(&self, sc: &Scene, view: &ViewMirror, node: usize, p: Pos2, lod: u8) -> Option<ActHit> {
        let n = sc.nodes.get(node)?;
        if lod < 2 || n.b.actions == 0 {
            return None;
        }
        let s = Self::scale(view);
        let r = self.rect_of(sc, view, node);
        if p.x < r.min.x || p.x > r.max.x {
            return None;
        }
        let y0 = r.min.y + n.b.actions_y * s;
        let rows = n.b.action_rows + usize::from(n.b.more);
        if p.y < y0 {
            return None;
        }
        let row = ((p.y - y0) / (ACTION_ROW * s)).floor();
        if row < 0.0 || row >= rows as f32 {
            return None;
        }
        let row = row as usize;
        Some(ActHit { node, row, more: row >= n.b.action_rows })
    }

    fn hit_header(&self, sc: &Scene, view: &ViewMirror, node: usize, p: Pos2) -> bool {
        let r = self.rect_of(sc, view, node);
        let h = r.min.y + HEADER * Self::scale(view);
        p.x >= r.min.x && p.x <= r.max.x && p.y >= r.min.y && p.y <= h
    }

    /// Where a dragged action would land: the node, the slot (exit?), the index it goes in at,
    /// and the y of the line that shows it (`DropTarget`).
    fn drop_target(
        &mut self,
        sc: &Scene,
        view: &ViewMirror,
        g: &Graph,
        p: Pos2,
        lod: u8,
    ) -> Option<(usize, bool, usize, f32)> {
        let node = self.hit_node(sc, view, p)?;
        let n = sc.nodes.get(node)?;
        if n.trigger || lod < 2 {
            return None;
        }
        let gn = g.find_node(&n.id)?;
        let s = Self::scale(view);
        let r = self.rect_of(sc, view, node);
        if self.hit_header(sc, view, node, p) {
            return Some((node, false, gn.on_enter.len(), r.min.y + HEADER * s));
        }
        let hit = self.hit_action(sc, view, node, p, lod)?;
        let top = r.min.y + (n.b.actions_y + ACTION_ROW * hit.row as f32) * s;
        if hit.more {
            let exit = !gn.on_exit.is_empty();
            return Some((node, exit, if exit { gn.on_exit.len() } else { gn.on_enter.len() }, top));
        }
        let row = n.action_at(hit.row)?;
        let (mut index, mut line) = (row.index, top);
        // the lower half of a row puts it after that row
        if p.y > top + ACTION_ROW * s * 0.5 {
            index += 1;
            line = top + ACTION_ROW * s;
        }
        Some((node, row.exit, index, line))
    }

    /// The screen shape of a link now: its bezier controls or its route.
    fn link_shape(
        &self,
        sc: &Scene,
        view: &ViewMirror,
        l: &SceneLink,
    ) -> (Pos2, Pos2, Pos2, Pos2, Vec<Pos2>) {
        let moved = self.moving()
            && (self.pos_of(sc, l.from as usize)
                != sc.nodes.get(l.from as usize).map_or([0.0; 2], |n| n.pos)
                || self.pos_of(sc, l.to as usize) != sc.nodes.get(l.to as usize).map_or([0.0; 2], |n| n.pos));
        let (w0, w3, path) = if moved {
            // a dragged end: the edge is laid out again here, against the nodes where they are drawn
            let (Some(fnode), Some(tnode)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize))
            else {
                return (Pos2::ZERO, Pos2::ZERO, Pos2::ZERO, Pos2::ZERO, Vec::new());
            };
            let pin = fnode.pins.iter().position(|p| *p == l.pin).unwrap_or(0);
            let w0 = layout::output_pin(self.pos_of(sc, l.from as usize), fnode.size, pin, fnode.pins.len());
            let w3 = layout::input_pin(self.pos_of(sc, l.to as usize), tnode.size);
            let path = if SceneLink::goes_back(w0, w3) {
                let boxes: Vec<[f32; 4]> = [l.from as usize, l.to as usize]
                    .iter()
                    .filter_map(|&i| {
                        sc.nodes.get(i).map(|n| {
                            let p = self.pos_of(sc, i);
                            [p[0], p[1], p[0] + n.size[0], p[1] + n.size[1]]
                        })
                    })
                    .collect();
                scene::route_back(&boxes, w0, w3, &mut Vec::new())
            } else {
                Vec::new()
            };
            (w0, w3, path)
        } else {
            (l.p0, l.p3, l.path.clone())
        };
        let p0 = self.to_screen(view, w0);
        let p3 = self.to_screen(view, w3);
        if path.is_empty() {
            let (c1, c2) = SceneLink::controls(w0, w3);
            (p0, self.to_screen(view, c1), self.to_screen(view, c2), p3, Vec::new())
        } else {
            (p0, p0, p3, p3, path.into_iter().map(|w| self.to_screen(view, w)).collect())
        }
    }

    fn visible_links(&mut self, sc: &Scene, r: [f32; 4]) -> Vec<usize> {
        self.scratch.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        sc.links_in(r, &mut scratch);
        let out = self.dedupe(sc.links.len(), &scratch);
        self.scratch = scratch;
        out
    }

    fn hit_link(&mut self, sc: &Scene, view: &ViewMirror, p: Pos2) -> Option<usize> {
        let r = LINK_GRAB + (2.0 * Self::scale(view)).max(1.0) * 0.5;
        let s = Self::scale(view);
        let w = self.to_world(view, p);
        let rw = r / s;
        let cands = self.visible_links(sc, [w[0] - rw, w[1] - rw, w[0] + rw, w[1] + rw]);
        let mut best = r * r;
        let mut hit = None;
        for i in cands {
            let Some(l) = sc.links.get(i) else { continue };
            let (p0, p1, p2, p3, path) = self.link_shape(sc, view, l);
            if path.is_empty() {
                let mut prev = p0;
                for k in 1..=16 {
                    let cur = bezier(p0, p1, p2, p3, k as f32 / 16.0);
                    let d = seg_dist2(p, prev, cur);
                    if d < best {
                        best = d;
                        hit = Some(i);
                    }
                    prev = cur;
                }
            } else {
                for k in 1..path.len() {
                    let (Some(&a), Some(&b)) = (path.get(k - 1), path.get(k)) else { continue };
                    let d = seg_dist2(p, a, b);
                    if d < best {
                        best = d;
                        hit = Some(i);
                    }
                }
            }
        }
        hit
    }

    // ---- the frame ----------------------------------------------------------------------------

    /// Draws the canvas into `rect` and handles its input.
    #[allow(clippy::too_many_lines)]
    pub fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &mut CanvasCx<'_>) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let ppp = ctx.pixels_per_point();
        self.origin = rect.min;
        self.size = vec2(rect.width().max(32.0), rect.height().max(32.0));
        self.drawn_frame = ctx.cumulative_frame_nr();
        cx.view.center = [layout::sane(cx.view.center[0]), layout::sane(cx.view.center[1])];
        if let Some(st) = cx.doc_status.take() {
            self.set_status(&ctx, &st);
        }
        // a commit in flight is drawn until the scene that holds it arrives
        if self.settle.as_ref().is_some_and(|s| cx.scene.is_some_and(|sc| sc.edit != s.from_edit)) {
            self.settle = None;
        }

        // deferred framing needs the rect and the scene
        if let Some(sc) = cx.scene {
            if self.want_frame_all {
                self.want_frame_all = false;
                self.frame_all_now(&ctx, sc, cx.view);
            }
            if self.want_frame_sel {
                self.want_frame_sel = false;
                self.frame_selection_now(&ctx, sc, cx.view);
            }
            // framing picks a zoom of its own; an explicit one was asked for later
            if let Some(z) = self.want_zoom.take() {
                cx.view.zoom = z;
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.canvas);
        let resp = ui.interact(rect, self.id, Sense::click_and_drag());
        let covered =
            ctx.input(|i| i.pointer.hover_pos()).is_some_and(|m| cx.covered.iter().any(|r| r.contains(m)));
        self.hovered = (resp.hovered() && !covered) || resp.dragged();
        let menu_up = egui::Popup::is_any_open(&ctx);
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            self.focused = true;
        } else if ctx.input(|i| i.pointer.any_pressed()) && !self.hovered && !menu_up {
            self.focused = false;
        }

        let lod = Self::lod(cx.view, ppp);
        let Some(sc) = cx.scene else {
            gz_ui::state::busy_line(
                &mut ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(16.0))),
                "Reading the graph",
            );
            return;
        };
        let mouse = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::new(-1.0e6, -1.0e6));
        let grab = Self::pin_grab(cx.view, ppp);
        self.update_hot(sc, cx, mouse, lod, grab, menu_up);
        self.tip.clear();

        self.draw_grid(&painter, &p, cx.view);
        let view_r = self.viewport(cx.view).unwrap_or([0.0; 4]);
        let pad = 64.0;
        let links =
            self.visible_links(sc, [view_r[0] - pad, view_r[1] - pad, view_r[2] + pad, view_r[3] + pad]);
        self.draw_links(&painter, &p, sc, cx, &links);
        self.draw_linking(&painter, &p, sc, cx.view, mouse);
        self.scratch.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        sc.nodes_in(view_r, &mut scratch);
        let mut visible = self.dedupe(sc.nodes.len(), &scratch);
        self.scratch = scratch;
        if self.moving() {
            if let Some(d) = &self.drag {
                visible.extend(d.nodes.iter().map(|(i, _, _)| *i));
            }
            visible.sort_unstable();
            visible.dedup();
        }
        self.visible = visible;
        let visible = std::mem::take(&mut self.visible);
        for &i in &visible {
            self.draw_node(&painter, &p, sc, cx, i, lod, mouse, ppp);
        }
        self.visible = visible;
        self.draw_action_drag(&painter, &p, sc, cx, mouse, lod);
        self.draw_marquee(&painter, &p, mouse);
        self.draw_minimap(&painter, &p, sc, cx, mouse);
        self.draw_beyond(&painter, &p, sc, cx.view, mouse);
        self.draw_status(&painter, &p, &ctx);
        if sc.nodes.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Right-click to add a node",
                Font::Body.id(),
                p.text_muted,
            );
        }

        self.handle_input(&ctx, sc, cx, mouse, lod, grab, &resp, menu_up);
        self.draw_menus(ui, sc, cx);
        if self.hovered && !self.tip.is_empty() && !menu_up {
            resp.on_hover_text_at_pointer(self.tip.clone());
        }
    }

    fn update_hot(&mut self, sc: &Scene, cx: &CanvasCx<'_>, mouse: Pos2, lod: u8, grab: f32, menu_up: bool) {
        self.hot_node = None;
        self.hot_pin = None;
        self.hot_link = None;
        self.hot_action = None;
        if !self.hovered || menu_up {
            return;
        }
        let view = &*cx.view;
        if let Some((from, _)) = &self.linking {
            let from = sc.node_of(from);
            let node =
                self.hit_node(sc, view, mouse).or_else(|| self.hit_node_near(sc, view, mouse, from, grab));
            if let Some(n) = node
                && Some(n) != from
                && sc.nodes.get(n).is_some_and(|x| !x.trigger)
            {
                self.hot_node = Some(n);
            }
            return;
        }
        if self.drag.as_ref().is_some_and(|d| d.moved)
            || self.marquee.is_some()
            || self.pan.is_some_and(|(_, m)| m)
            || self.act_drag.as_ref().is_some_and(|a| a.dragging)
        {
            return;
        }
        let mut best = f32::MAX;
        for i in self.candidates(sc, view, mouse, grab) {
            if let Some((k, d2)) = self.hit_pin(sc, view, i, mouse, grab)
                && d2 < best
            {
                best = d2;
                self.hot_node = Some(i);
                self.hot_pin = Some(k);
            }
            if sc.nodes.get(i).is_some_and(|n| !n.trigger)
                && let Some(d2) = Self::near_pin(mouse, self.input_pin(sc, view, i), 1.0, grab)
                && d2 < best
            {
                best = d2;
                self.hot_node = Some(i);
                self.hot_pin = None;
            }
        }
        let over = self.hit_node(sc, view, mouse);
        if self.hot_node.is_none()
            && let Some(o) = over
        {
            self.hot_action = self.hit_action(sc, view, o, mouse, lod);
        }
        // a pin wins over a link it starts from, and a node body swallows the wires behind it
        if self.hot_node.is_none() && over.is_none() {
            self.hot_link = self.hit_link(sc, view, mouse);
        }
    }

    fn draw_grid(&self, painter: &Painter, p: &Palette, view: &ViewMirror) {
        let s = Self::scale(view);
        let mut step = GRID * 4.0 * s;
        while step < 24.0 {
            step *= 2.0;
        }
        let o = self.to_screen(view, [0.0, 0.0]);
        let sx = o.x - ((o.x - self.origin.x) / step).floor() * step;
        let sy = o.y - ((o.y - self.origin.y) / step).floor() * step;
        if !sx.is_finite() || !sy.is_finite() {
            return;
        }
        let nx = (self.size.x / step) as i32 + 2;
        let ny = (self.size.y / step) as i32 + 2;
        if !(1..=4096).contains(&nx) || !(1..=4096).contains(&ny) {
            return;
        }
        let stroke = Stroke::new(1.0, Palette::alpha(p.border_subtle, 0.45));
        let (x1, y1) = (self.origin.x + self.size.x, self.origin.y + self.size.y);
        for i in 0..nx {
            let x = sx + i as f32 * step;
            if x >= x1 {
                break;
            }
            if x >= self.origin.x {
                painter.line_segment([pos2(x, self.origin.y), pos2(x, y1)], stroke);
            }
        }
        for i in 0..ny {
            let y = sy + i as f32 * step;
            if y >= y1 {
                break;
            }
            if y >= self.origin.y {
                painter.line_segment([pos2(self.origin.x, y), pos2(x1, y)], stroke);
            }
        }
    }

    fn draw_links(&self, painter: &Painter, p: &Palette, sc: &Scene, cx: &CanvasCx<'_>, links: &[usize]) {
        let s = Self::scale(cx.view);
        let base = (2.0 * s).max(1.0);
        for &i in links {
            let Some(l) = sc.links.get(i) else { continue };
            let sel = sc.nodes.get(l.from as usize).is_some_and(|n| cx.view.is_selected(&n.id))
                || sc.nodes.get(l.to as usize).is_some_and(|n| cx.view.is_selected(&n.id));
            let hot = self.hot_link == Some(i);
            let col = if hot {
                p.text
            } else if sel {
                p.accent_bright
            } else {
                p.text_muted
            };
            let (p0, p1, p2, p3, path) = self.link_shape(sc, cx.view, l);
            let stroke = |c: Color32, w: f32| {
                if path.is_empty() {
                    painter.add(Shape::CubicBezier(CubicBezierShape::from_points_stroke(
                        [p0, p1, p2, p3],
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(w, c),
                    )));
                } else {
                    painter.add(Shape::Path(PathShape::line(path.clone(), Stroke::new(w, c))));
                }
            };
            let mut th = base;
            if hot {
                stroke(Palette::alpha(p.accent_bright, 0.35), th * 3.5 + 2.0);
                th *= 1.8;
            }
            stroke(col, th);
            // the arrow head along the last piece of the wire: a routed edge slants in
            let a = (7.0 * s).max(4.0) * if hot { 1.35 } else { 1.0 };
            let mut dir = vec2(0.0, 1.0);
            if path.len() >= 2
                && let Some(prev) = path.get(path.len() - 2)
            {
                let d = p3 - *prev;
                if d.length() > 0.001 {
                    dir = d.normalized();
                }
            }
            let b = p3 - dir * a;
            let half = vec2(-dir.y, dir.x) * a * 0.6;
            painter.add(Shape::convex_polygon(vec![p3, b + half, b - half], col, Stroke::NONE));
        }
    }

    fn draw_linking(&self, painter: &Painter, p: &Palette, sc: &Scene, view: &ViewMirror, mouse: Pos2) {
        let Some((from, pin)) = &self.linking else { return };
        let Some(i) = sc.node_of(from) else { return };
        let Some(k) = sc.nodes.get(i).and_then(|n| n.pins.iter().position(|x| x == pin.as_str())) else {
            return;
        };
        let p0 = self.output_pin(sc, view, i, k);
        let d = (scene::LINK_TANGENT * Self::scale(view)).max((mouse.y - p0.y).abs() * 0.4);
        painter.add(Shape::CubicBezier(CubicBezierShape::from_points_stroke(
            [p0, pos2(p0.x, p0.y + d), pos2(mouse.x, mouse.y - d), mouse],
            false,
            Color32::TRANSPARENT,
            Stroke::new(2.0, p.accent_bright),
        )));
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn draw_node(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        i: usize,
        lod: u8,
        mouse: Pos2,
        ppp: f32,
    ) {
        let Some(n) = sc.nodes.get(i) else { return };
        let view = &*cx.view;
        let s = Self::scale(view);
        let r = self.rect_of(sc, view, i);
        let (a, b) = (r.min, r.max);
        let selected = view.is_selected(&n.id);
        // an unreachable node is dimmed, never hidden: it is still the author's to wire
        let alpha = if n.reachable { 1.0 } else { 0.45 };
        let header = HEADER * s;
        let rnd = (if n.trigger { HEADER * 0.5 } else { 6.0 }) * s;
        let header_only = n.b.h <= HEADER + PAD + 0.5;
        let hb = pos2(b.x, if header_only { b.y } else { a.y + header });
        let head_corners = if header_only {
            radius(rnd)
        } else {
            CornerRadius { nw: radius(rnd).nw, ne: radius(rnd).ne, sw: 0, se: 0 }
        };
        let fam = |f: Family, k: f32| look::family_color(p, f, k);

        // the card: body, header tinted by the family, the family line under it
        painter.rect_filled(r, radius(rnd), Palette::alpha(p.raised, alpha));
        let head = Rect::from_min_max(a, hb);
        painter.rect_filled(head, head_corners, Palette::alpha(p.hover, alpha));
        painter.rect_filled(head, head_corners, fam(n.family, 0.20 * alpha));
        if !header_only {
            painter.line_segment(
                [pos2(a.x + rnd * 0.5, hb.y), pos2(b.x - rnd * 0.5, hb.y)],
                Stroke::new((2.0 * s).max(1.0), fam(n.family, alpha)),
            );
        }

        let small = Self::font(Font::Small, SMALL_PT, view, ppp);
        if lod >= 1 {
            let inset = INSET * s;
            let strong = Self::font(Font::Strong, TITLE_PT, view, ppp);
            let cy = a.y + header * 0.5;
            let mut x = a.x + inset;
            let mut right = b.x - inset;
            if lod == 2 {
                let icon = 16.0 * s;
                icons::draw(
                    painter,
                    n.icon,
                    Rect::from_min_size(pos2(x, cy - icon * 0.5), Vec2::splat(icon)),
                    fam(n.family, alpha),
                );
                x += icon + 6.0 * s;
                // badges from the right: problems, the bookmark, "once"
                let badge = 14.0 * s;
                if let Some((sev, msg)) = cx.data.severity(&n.id) {
                    let bm = Rect::from_min_size(pos2(right - badge, cy - badge * 0.5), Vec2::splat(badge));
                    icons::draw(
                        painter,
                        if *sev == 2 { Icon::Error } else { Icon::Warning },
                        bm,
                        Palette::alpha(if *sev == 2 { p.error } else { p.warning }, alpha),
                    );
                    if self.hovered && bm.contains(mouse) {
                        msg.clone_into(&mut self.tip);
                    }
                    right -= badge + 4.0 * s;
                }
                if cx.bookmarks.contains(&n.id) {
                    let bm = Rect::from_min_size(pos2(right - badge, cy - badge * 0.5), Vec2::splat(badge));
                    icons::draw(painter, Icon::BookmarkFilled, bm, Palette::alpha(p.accent_bright, alpha));
                    right -= badge + 4.0 * s;
                }
                if n.once {
                    let tw = gz_ui::text::width(painter.ctx(), &small, "once");
                    painter.text(
                        pos2(right - tw, cy),
                        Align2::LEFT_CENTER,
                        "once",
                        small.clone(),
                        Palette::alpha(p.text_muted, alpha),
                    );
                    right -= tw + 6.0 * s;
                }
                // the id gets what the title leaves, and never more than 40% of the row
                let room = (right - x).max(0.0);
                let title_w = gz_ui::text::width(painter.ctx(), &strong, &n.title);
                let id_w = gz_ui::text::width(painter.ctx(), &small, &n.id)
                    .min((room - title_w - 8.0 * s).max(room * 0.4));
                let line_px = small.size / 0.75;
                if id_w > line_px * 1.5 {
                    paint_fit(
                        painter,
                        &small,
                        pos2(right - id_w, cy),
                        Palette::alpha(p.text_muted, alpha),
                        &n.id,
                        id_w,
                    );
                }
                paint_fit(
                    painter,
                    &strong,
                    pos2(x, cy),
                    Palette::alpha(p.text, alpha),
                    &n.title,
                    if id_w > line_px * 1.5 { room - id_w - 8.0 * s } else { room },
                );
            } else {
                paint_fit(painter, &strong, pos2(x, cy), Palette::alpha(p.text, alpha), &n.title, right - x);
            }
        }

        // the body: lines and actions in words at full detail, placeholder bars below it
        if lod == 2 {
            let body = Self::font(Font::Body, LINE_PT, view, ppp);
            let inset = INSET * s;
            let inner = n.size[0] * s - inset * 2.0;
            for (k, line) in n.lines.iter().take(n.b.lines).enumerate() {
                let ly = a.y + (n.b.lines_y + LINE * k as f32 + LINE * 0.5) * s;
                if line.key.is_empty() {
                    paint_fit(
                        painter,
                        &small,
                        pos2(a.x + inset, ly),
                        Palette::alpha(p.text_secondary, alpha),
                        &line.value,
                        inner,
                    );
                    continue;
                }
                // a parameter reads "name: value", a condition "Title what"
                let key = if line.cond.is_empty() { format!("{}:", line.key) } else { line.key.clone() };
                let share = if line.value.is_empty() {
                    1.0
                } else if line.cond.is_empty() {
                    0.45
                } else {
                    0.6
                };
                let kw = paint_fit(
                    painter,
                    &body,
                    pos2(a.x + inset, ly),
                    Palette::alpha(p.text_muted, alpha),
                    &key,
                    inner * share,
                );
                let vx = a.x + inset + kw + 6.0 * s;
                paint_fit(
                    painter,
                    &body,
                    pos2(vx, ly),
                    Palette::alpha(p.text, alpha),
                    &line.value,
                    b.x - inset - vx,
                );
            }
            if n.b.actions > 0 {
                self.draw_actions(painter, p, n, cx, i, a, alpha, &body, &small);
            }
        } else {
            let bar = Palette::alpha(p.border_subtle, alpha);
            let inset = INSET * s;
            for k in 0..n.b.lines {
                let ly = a.y + (n.b.lines_y + LINE * (k as f32 + 0.3)) * s;
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(a.x + inset, ly),
                        vec2((n.size[0] * s - inset * 2.0) * 0.6, LINE * 0.4 * s),
                    ),
                    radius(2.0 * s),
                    bar,
                );
            }
            for k in 0..n.b.action_rows {
                let ly = a.y + (n.b.actions_y + ACTION_ROW * (k as f32 + 0.3)) * s;
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(a.x + inset, ly),
                        vec2((n.size[0] * s - inset * 2.0) * 0.8, ACTION_ROW * 0.4 * s),
                    ),
                    radius(2.0 * s),
                    bar,
                );
            }
        }

        let hovered_node = self.hovered && r.contains(mouse);
        let border = if selected {
            Stroke::new(2.0, p.accent_bright)
        } else if hovered_node {
            Stroke::new(1.0, Palette::alpha(p.border_strong, alpha))
        } else {
            Stroke::new(1.0, Palette::alpha(p.border, alpha))
        };
        painter.rect_stroke(r, radius(rnd), border, StrokeKind::Inside);

        // the pins sit on the edges; their names are inside the node, where no wire runs
        let pr = (2.0 / ppp).max(PIN_R * s);
        let halo = Self::pin_grab(view, ppp);
        let slot_w = n.size[0] * s / (n.pins.len() + 1) as f32;
        for (k, pin) in n.pins.iter().enumerate() {
            let c = self.output_pin(sc, view, i, k);
            let hot = self.linking.as_ref().is_some_and(|(f, pn)| *f == n.id && pn == pin);
            let over = self.hot_node == Some(i) && self.hot_pin == Some(k);
            if lod == 2 && n.b.pin_labels {
                let (shown, _) =
                    gz_ui::text::ellipsize(painter.ctx(), &small, pin, slot_w - 4.0 * s, gz_ui::Clip::End);
                let ly = a.y + (n.b.pins_y + PIN_ROW * 0.5) * s;
                painter.text(
                    pos2(c.x, ly),
                    Align2::CENTER_CENTER,
                    shown,
                    small.clone(),
                    if hot || over {
                        Palette::alpha(p.accent_bright, alpha)
                    } else {
                        Palette::alpha(p.text_secondary, alpha)
                    },
                );
            }
            if over {
                painter.circle_filled(c, halo, Palette::alpha(p.accent_bright, 0.15));
            }
            let rr = if over { pr * 1.4 } else { pr };
            painter.circle_filled(
                c,
                rr,
                if hot || over { p.accent_bright } else { Palette::alpha(p.text_secondary, alpha) },
            );
            painter.circle_stroke(c, rr, Stroke::new(1.0, p.canvas));
        }
        if !n.trigger {
            let c = self.input_pin(sc, view, i);
            let over = self.hot_node == Some(i) && self.hot_pin.is_none();
            if over {
                painter.circle_filled(c, halo, Palette::alpha(p.accent_bright, 0.15));
            }
            let rr = if over { pr * 1.4 } else { pr };
            painter.circle_filled(
                c,
                rr,
                if over { p.accent_bright } else { Palette::alpha(p.text_secondary, alpha) },
            );
            painter.circle_stroke(c, rr, Stroke::new(1.0, p.canvas));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_actions(
        &self,
        painter: &Painter,
        p: &Palette,
        n: &SceneNode,
        cx: &CanvasCx<'_>,
        index: usize,
        a: Pos2,
        alpha: f32,
        body: &FontId,
        small: &FontId,
    ) {
        let view = &*cx.view;
        let s = Self::scale(view);
        let w = n.size[0] * s;
        let row_h = ACTION_ROW * s;
        let y0 = a.y + n.b.actions_y * s;
        let inset = INSET * s;
        let one_selected = view.is_selected(&n.id) && view.selection.len() == 1;
        if n.b.lines > 0 {
            painter.line_segment(
                [pos2(a.x + inset, y0 - PAD * 0.5 * s), pos2(a.x + w - inset, y0 - PAD * 0.5 * s)],
                Stroke::new(1.0, Palette::alpha(p.border_subtle, alpha)),
            );
        }
        for (r, act) in n.actions.iter().enumerate() {
            let ra = pos2(a.x + 3.0 * s, y0 + row_h * r as f32);
            let rb = pos2(a.x + w - 3.0 * s, y0 + row_h * (r + 1) as f32);
            let row = Rect::from_min_max(ra, rb);
            let selected = one_selected && view.slot == format!("{}:{}", slot_name(act.exit), act.index);
            let hot = self.hot_action.is_some_and(|h| h.node == index && h.row == r && !h.more);
            let dragged = self
                .act_drag
                .as_ref()
                .is_some_and(|d| d.dragging && d.node == n.id && d.exit == act.exit && d.index == act.index);
            if selected {
                painter.rect_filled(row, radius(3.0 * s), Palette::alpha(p.selection_focused, alpha));
            } else if hot {
                painter.rect_filled(row, radius(3.0 * s), Palette::alpha(p.hover, alpha));
            }
            if dragged {
                painter.rect_stroke(
                    row,
                    radius(3.0 * s),
                    Stroke::new((1.5 * s).max(1.0), p.accent_bright),
                    StrokeKind::Inside,
                );
            }
            let mut x = a.x + inset;
            let icon = 14.0 * s;
            let cy = row.center().y;
            icons::draw(
                painter,
                act.icon,
                Rect::from_min_size(pos2(x, cy - icon * 0.5), Vec2::splat(icon)),
                look::family_color(p, act.family, alpha),
            );
            x += icon + 6.0 * s;
            let mut right = a.x + w - inset;
            if act.exit {
                let tw = gz_ui::text::width(painter.ctx(), small, "exit");
                painter.text(
                    pos2(right - tw, cy),
                    Align2::LEFT_CENTER,
                    "exit",
                    small.clone(),
                    Palette::alpha(p.text_muted, alpha),
                );
                right -= tw + 6.0 * s;
            }
            let tw =
                paint_fit(painter, body, pos2(x, cy), Palette::alpha(p.text, alpha), &act.title, right - x);
            let sx = x + tw + 6.0 * s;
            if right - sx > body.size / 0.75 * 3.0 {
                paint_fit(
                    painter,
                    body,
                    pos2(sx, cy),
                    Palette::alpha(p.text_muted, alpha),
                    &act.summary,
                    right - sx,
                );
            }
        }
        if n.b.more {
            let top = y0 + row_h * n.b.action_rows as f32;
            let hot = self.hot_action.is_some_and(|h| h.node == index && h.more);
            if hot {
                painter.rect_filled(
                    Rect::from_min_max(pos2(a.x + 3.0 * s, top), pos2(a.x + w - 3.0 * s, top + row_h)),
                    radius(3.0 * s),
                    Palette::alpha(p.hover, alpha),
                );
            }
            let more = format!("+{} more", n.b.actions - n.b.action_rows);
            painter.text(
                pos2(a.x + inset, top + row_h * 0.5),
                Align2::LEFT_CENTER,
                more,
                small.clone(),
                Palette::alpha(p.text_secondary, alpha),
            );
        }
    }

    fn draw_marquee(&self, painter: &Painter, p: &Palette, mouse: Pos2) {
        let Some(from) = self.marquee else { return };
        let r = Rect::from_two_pos(from, mouse);
        painter.rect_filled(r, CornerRadius::ZERO, Palette::alpha(p.accent_bright, 0.12));
        painter.rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(1.0, Palette::alpha(p.accent_bright, 0.8)),
            StrokeKind::Inside,
        );
    }

    fn draw_action_drag(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        mouse: Pos2,
        lod: u8,
    ) {
        let Some(d) = self.act_drag.clone().filter(|d| d.dragging) else { return };
        let Some(gn) = cx.graph.find_node(&d.node) else { return };
        let list = if d.exit { &gn.on_exit } else { &gn.on_enter };
        let Some(act) = list.get(d.index) else { return };
        let target = self.drop_target(sc, cx.view, cx.graph, mouse, lod);
        if let Some((node, _, _, line)) = target {
            let r = self.rect_of(sc, cx.view, node);
            let s = Self::scale(cx.view);
            painter.line_segment(
                [pos2(r.min.x + 4.0 * s, line), pos2(r.max.x - 4.0 * s, line)],
                Stroke::new(2.0, p.accent_bright),
            );
        }
        // red over anything that is not a place for it: releasing there cancels the move
        let title = look::kind_title(cx.catalog, &act.kind, cx.graph.is_behaviour());
        let w = gz_ui::text::width(painter.ctx(), &Font::Body.id(), &title).min(200.0) + 34.0;
        let pill = Rect::from_min_size(pos2(mouse.x + 12.0, mouse.y + 4.0), vec2(w, 24.0));
        painter.rect_filled(pill, CornerRadius::same(4), Palette::alpha(p.overlay, 0.95));
        painter.rect_stroke(
            pill,
            CornerRadius::same(4),
            Stroke::new(1.0, if target.is_some() { p.accent_bright } else { p.error }),
            StrokeKind::Inside,
        );
        let icon = look::kind_icon(cx.catalog, &act.kind);
        icons::draw(
            painter,
            icon,
            Rect::from_min_size(pos2(pill.min.x + 6.0, pill.center().y - 7.0), Vec2::splat(14.0)),
            look::family_color(p, look::family_of(icon), 1.0),
        );
        paint_fit(
            painter,
            &Font::Body.id(),
            pos2(pill.min.x + 26.0, pill.center().y),
            p.text,
            &title,
            w - 32.0,
        );
    }

    /// 180 x 120 pt in the bottom-right corner, only for a graph big enough to get lost in
    /// (`DrawMinimap`). The picture is the scene's; a frame translates a copy of it once per
    /// place and adds the view and the selection.
    fn draw_minimap(&mut self, painter: &Painter, p: &Palette, sc: &Scene, cx: &CanvasCx<'_>, mouse: Pos2) {
        self.minimap_hovered = false;
        self.minimap_shown = false;
        let margin = 8.0;
        let box_size = vec2(MINIMAP_W, MINIMAP_H);
        if !cx.minimap || sc.nodes.len() < doc::MINIMAP_MIN_NODES {
            return;
        }
        if self.size.x < box_size.x * 2.0 + margin * 2.0 || self.size.y < box_size.y * 2.0 + margin * 2.0 {
            return;
        }
        let Some(mm) = &sc.minimap else { return };
        self.minimap_shown = true;
        let bmax = pos2(self.origin.x + self.size.x - margin, self.origin.y + self.size.y - margin);
        let bmin = bmax - box_size;
        let boxr = Rect::from_min_max(bmin, bmax);
        painter.rect_filled(boxr, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
        painter.rect_stroke(boxr, CornerRadius::same(6), Stroke::new(1.0, p.border), StrokeKind::Inside);
        let mesh = match &self.minimap_cache {
            Some((edit, at, m)) if *edit == sc.edit && *at == bmin => m.clone(),
            _ => {
                let mut m = mm.mesh.clone();
                m.translate(bmin.to_vec2());
                let m = Arc::new(m);
                self.minimap_cache = Some((sc.edit, bmin, m.clone()));
                m
            }
        };
        painter.add(Shape::Mesh(mesh));
        let local = |w: [f32; 2]| mm.to_local(w) + bmin.to_vec2();
        for id in &cx.view.selection {
            if let Some(n) = sc.node_of(id).and_then(|i| sc.nodes.get(i)) {
                let r = Rect::from_min_max(local(n.pos), local(add(n.pos, n.size)));
                painter.rect_stroke(
                    r.expand(0.5),
                    CornerRadius::same(1),
                    Stroke::new(1.5, p.text),
                    StrokeKind::Outside,
                );
            }
        }
        for id in cx.bookmarks {
            if cx.view.is_selected(id) {
                continue;
            }
            if let Some(n) = sc.node_of(id).and_then(|i| sc.nodes.get(i)) {
                let r = Rect::from_min_max(local(n.pos), local(add(n.pos, n.size)));
                painter.rect_stroke(
                    r.expand(0.5),
                    CornerRadius::same(1),
                    Stroke::new(1.5, p.accent_bright),
                    StrokeKind::Outside,
                );
            }
        }
        if let Some(v) = self.viewport(cx.view) {
            let fitted = mm.fitted.translate(bmin.to_vec2());
            let clamp =
                |q: Pos2| pos2(q.x.clamp(fitted.min.x, fitted.max.x), q.y.clamp(fitted.min.y, fitted.max.y));
            let vr = Rect::from_min_max(clamp(local([v[0], v[1]])), clamp(local([v[2], v[3]])));
            painter.rect_stroke(
                vr,
                CornerRadius::ZERO,
                Stroke::new(1.5, p.accent_bright),
                StrokeKind::Middle,
            );
        }
        self.minimap_hovered = self.hovered && boxr.contains(mouse);
        if self.minimap_hovered {
            self.tip = "Minimap: click or drag to move the view".to_owned();
        }
    }

    /// Where the graph goes on past the view, when no minimap says it: a pill at the middle of each
    /// edge with the nodes wholly beyond it (`DrawBeyond`). The counts are kept either way.
    fn draw_beyond(&mut self, painter: &Painter, p: &Palette, sc: &Scene, view: &ViewMirror, mouse: Pos2) {
        self.beyond_hot = None;
        self.beyond = [0; 4];
        self.beyond_near = [None; 4];
        let Some(v) = self.viewport(view) else { return };
        if sc.nodes.is_empty() {
            return;
        }
        let (counts, near) = sc.beyond(v);
        self.beyond = counts;
        self.beyond_near = near;
        if self.minimap_shown {
            return;
        }
        const ICONS: [Icon; 4] = [Icon::ChevronUp, Icon::ChevronDown, Icon::ChevronLeft, Icon::ChevronRight];
        const WHERE: [&str; 4] = ["above", "below", "left of", "right of"];
        let font = Font::Small.id();
        let icon = 12.0;
        let (padx, pady) = (8.0, 4.0);
        let inset = 8.0;
        for side in 0..4 {
            let count = counts.get(side).copied().unwrap_or(0);
            if count == 0 {
                continue;
            }
            let label = format!("{count} more");
            let h = round(Font::Small.line().max(icon) + pady * 2.0);
            let w = round(padx + icon + 4.0 + gz_ui::text::width(painter.ctx(), &font, &label) + padx);
            let a = match side {
                0 => pos2(self.origin.x + (self.size.x - w) * 0.5, self.origin.y + inset),
                1 => pos2(self.origin.x + (self.size.x - w) * 0.5, self.origin.y + self.size.y - inset - h),
                2 => pos2(self.origin.x + inset, self.origin.y + (self.size.y - h) * 0.5),
                _ => pos2(self.origin.x + self.size.x - inset - w, self.origin.y + (self.size.y - h) * 0.5),
            };
            let r = Rect::from_min_size(pos2(round(a.x), round(a.y)), vec2(w, h));
            let hot = self.hovered && r.contains(mouse);
            if hot {
                self.beyond_hot = Some(side);
            }
            painter.rect_filled(r, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
            if hot {
                painter.rect_filled(r, CornerRadius::same(6), p.hover);
            }
            painter.rect_stroke(
                r,
                CornerRadius::same(6),
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
            if let Some(ic) = ICONS.get(side) {
                icons::draw(
                    painter,
                    *ic,
                    Rect::from_min_size(pos2(r.min.x + padx, r.center().y - icon * 0.5), Vec2::splat(icon)),
                    p.text_secondary,
                );
            }
            painter.text(
                pos2(r.min.x + padx + icon + 4.0, r.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                p.text_secondary,
            );
            if hot {
                self.tip = format!(
                    "{count} {} {} the view. Click to bring the nearest one in",
                    if count == 1 { "node" } else { "nodes" },
                    WHERE.get(side).copied().unwrap_or("")
                );
            }
        }
    }

    fn draw_status(&mut self, painter: &Painter, p: &Palette, ctx: &egui::Context) {
        if self.status.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.status_at > STATUS_SECONDS {
            self.status.clear();
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let font = Font::Body.id();
        let (shown, _) =
            gz_ui::text::ellipsize(ctx, &font, &self.status, self.size.x - 40.0, gz_ui::Clip::End);
        let w = gz_ui::text::width(ctx, &font, &shown);
        let r = Rect::from_min_size(self.origin + vec2(8.0, 8.0), vec2(w + 20.0, Font::Body.line() + 8.0));
        painter.rect_filled(r, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
        painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
        painter.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, shown, font, p.text_secondary);
    }

    // ---- input --------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn handle_input(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        lod: u8,
        grab: f32,
        resp: &egui::Response,
        menu_up: bool,
    ) {
        let (
            pressed,
            down,
            released,
            sec_pressed,
            sec_down,
            sec_released,
            mid_down,
            delta,
            mods,
            wheel,
            double,
            now,
        ) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.button_pressed(PointerButton::Secondary),
                i.pointer.button_down(PointerButton::Secondary),
                i.pointer.button_released(PointerButton::Secondary),
                i.pointer.button_down(PointerButton::Middle),
                i.pointer.delta(),
                i.modifiers,
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { delta, .. } => delta.y,
                        _ => 0.0,
                    })
                    .sum::<f32>(),
                i.pointer.button_double_clicked(PointerButton::Primary),
                i.time,
            )
        });
        let covered = cx.covered.iter().any(|r| r.contains(mouse));
        let over = (resp.hovered() || resp.contains_pointer()) && !menu_up && !covered;
        // the minimap and the edge pills are the canvas's own controls: a click there is theirs
        let hovered = over && !self.minimap_hovered && self.beyond_hot.is_none();

        if over && pressed {
            if let Some(side) = self.beyond_hot {
                self.reveal_beyond(sc, cx.view, side);
            }
            if self.minimap_hovered {
                self.minimap_panning = true;
            }
        }
        if self.minimap_panning {
            if down {
                if let Some(mm) = &sc.minimap {
                    let bmin = pos2(
                        self.origin.x + self.size.x - 8.0 - MINIMAP_W,
                        self.origin.y + self.size.y - 8.0 - MINIMAP_H,
                    );
                    let w = mm.to_world(mouse - bmin.to_vec2());
                    cx.view.center = [layout::sane(w[0]), layout::sane(w[1])];
                    self.cancel_framing();
                }
            } else {
                self.minimap_panning = false;
            }
        }

        // the wheel zooms around the cursor
        if hovered && wheel != 0.0 {
            self.zoom_by(cx.view, if wheel > 0.0 { 1 } else { -1 }, Some(mouse));
            self.cancel_framing();
            self.last_wheel = now;
        }

        // the right button pans while dragging, opens the menu of what is under it on a click
        if hovered && sec_pressed {
            self.pan = Some((mouse, false));
        }
        if let Some((start, moved)) = self.pan {
            if sec_down {
                if !moved && (mouse.x - start.x).abs() + (mouse.y - start.y).abs() > 3.0 {
                    self.pan = Some((start, true));
                }
                if self.pan.is_some_and(|(_, m)| m) && delta != Vec2::ZERO {
                    let s = Self::scale(cx.view);
                    cx.view.center = [cx.view.center[0] - delta.x / s, cx.view.center[1] - delta.y / s];
                    self.cancel_framing();
                }
            }
            if sec_released {
                self.pan = None;
                if !moved && hovered {
                    self.open_context_menu(ctx, sc, cx, mouse, lod);
                }
            }
        }
        // the middle button pans too
        if hovered && mid_down && delta != Vec2::ZERO {
            self.middle_pan = true;
            let s = Self::scale(cx.view);
            cx.view.center = [cx.view.center[0] - delta.x / s, cx.view.center[1] - delta.y / s];
            self.cancel_framing();
        }
        if !mid_down {
            self.middle_pan = false;
        }

        // the left button
        if hovered && pressed {
            let mut node = self.hit_node(sc, cx.view, mouse);
            let mut pin = node.and_then(|n| self.hit_pin(sc, cx.view, n, mouse, grab)).map(|(k, _)| k);
            if node.is_none()
                && let Some((n, k)) = self.hit_any_pin(sc, cx.view, mouse, grab)
            {
                node = Some(n);
                pin = Some(k);
            }
            let act = node.and_then(|n| self.hit_action(sc, cx.view, n, mouse, lod));
            match (node, pin, act) {
                (Some(n), Some(k), _) => {
                    if let Some(sn) = sc.nodes.get(n)
                        && let Some(pn) = sn.pins.get(k)
                    {
                        self.linking = Some((sn.id.clone(), SmolStr::new(pn)));
                    }
                }
                (Some(n), None, Some(hit)) => {
                    if let Some(sn) = sc.nodes.get(n) {
                        cx.view.select_only(&sn.id);
                        if hit.more {
                            // the first action the node has no room for opens in the Details panel
                            if let Some((exit, index)) = sn.more_target {
                                cx.view.slot = format!("{}:{index}", slot_name(exit));
                            }
                            self.want_focus_action = true;
                        } else if let Some(row) = sn.action_at(hit.row) {
                            cx.view.slot = format!("{}:{}", slot_name(row.exit), row.index);
                            self.act_drag = Some(ActDrag {
                                node: sn.id.clone(),
                                exit: row.exit,
                                index: row.index,
                                start: mouse,
                                dragging: false,
                            });
                            if double {
                                self.want_focus_action = true;
                            }
                        }
                    }
                }
                (Some(n), None, None) => {
                    if let Some(sn) = sc.nodes.get(n) {
                        let id = sn.id.clone();
                        if mods.ctrl || mods.command {
                            cx.view.toggle(&id);
                        } else if !cx.view.is_selected(&id) {
                            cx.view.select_only(&id);
                        } else {
                            cx.view.slot.clear();
                        }
                        let nodes: Vec<(usize, SmolStr, [f32; 2])> = cx
                            .view
                            .selection
                            .iter()
                            .filter_map(|s| sc.node_of(s).map(|i| (i, s.clone(), self.pos_of(sc, i))))
                            .collect();
                        self.drag = Some(Drag { nodes, start: mouse, moved: false, delta: [0.0, 0.0] });
                    }
                }
                (None, _, _) => {
                    // the lit one: what Alt+LMB removes is what the halo pointed at
                    if self.hot_link.is_some() && mods.alt {
                        self.delete_hot_link(ctx, sc, cx);
                    } else {
                        self.marquee = Some(mouse);
                        if !(mods.ctrl || mods.command) {
                            cx.view.selection.clear();
                            cx.view.slot.clear();
                        }
                    }
                }
            }
        }

        // dragging nodes
        let s = Self::scale(cx.view);
        let mut drag_over = false;
        if let Some(d) = &mut self.drag {
            if down {
                let dd = mouse - d.start;
                if dd.x.abs() + dd.y.abs() > 2.0 {
                    d.moved = true;
                }
                d.delta = [dd.x / s, dd.y / s];
            }
            drag_over = released || !down;
        }
        if drag_over {
            self.end_drag(cx);
        }

        // an action being dragged: a press only nominates it, the move starts once the mouse travelled
        let mut act_over = None;
        if let Some(a) = &mut self.act_drag {
            if down && (mouse - a.start).length() > 6.0 {
                a.dragging = true;
            }
            if released || !down {
                act_over = Some(a.clone());
            }
        }
        if let Some(a) = act_over {
            self.act_drag = None;
            if a.dragging
                && let Some((node, exit, at, _)) = self.drop_target(sc, cx.view, cx.graph, mouse, lod)
                && let Some(to) = sc.nodes.get(node)
            {
                move_action(cx, &a.node, a.exit, a.index, &to.id, exit, at);
            }
        }

        if self.linking.is_some() && (released || !down) {
            self.end_link(ctx, sc, cx, mouse, grab);
        }
        if let Some(from) = self.marquee
            && (released || !down)
        {
            self.marquee = None;
            let r = Rect::from_two_pos(from, mouse);
            if r.width() >= 2.0 || r.height() >= 2.0 {
                let a = self.to_world(cx.view, r.min);
                let b = self.to_world(cx.view, r.max);
                self.scratch.clear();
                let mut scratch = std::mem::take(&mut self.scratch);
                sc.nodes_in([a[0], a[1], b[0], b[1]], &mut scratch);
                let found = self.dedupe(sc.nodes.len(), &scratch);
                self.scratch = scratch;
                for i in found {
                    if !self.rect_of(sc, cx.view, i).intersects(r) {
                        continue;
                    }
                    if let Some(n) = sc.nodes.get(i)
                        && !cx.view.is_selected(&n.id)
                    {
                        cx.view.selection.push(n.id.clone());
                    }
                }
            }
        }

        // what the view does is the document's once the gesture is over
        let wheel_busy = now - self.last_wheel < WHEEL_SETTLE;
        self.view_moving =
            self.pan.is_some_and(|(_, m)| m) || self.middle_pan || self.minimap_panning || wheel_busy;
        if wheel_busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // what the cursor is on, said on hover
        if hovered
            && self.tip.is_empty()
            && self.drag.is_none()
            && self.linking.is_none()
            && self.marquee.is_none()
            && self.act_drag.as_ref().is_none_or(|a| !a.dragging)
        {
            if let Some(h) = self.hot_action {
                if h.more {
                    self.tip = "Show the rest of the actions in the Details panel".to_owned();
                } else if let Some(sn) = sc.nodes.get(h.node)
                    && let Some(row) = sn.action_at(h.row)
                    && let Some(gn) = cx.graph.find_node(&sn.id)
                    && let Some(a) = (if row.exit { &gn.on_exit } else { &gn.on_enter }).get(row.index)
                {
                    let mut t = format!(
                        "{} {}: {}",
                        if row.exit { "On exit" } else { "On enter" },
                        row.index + 1,
                        row.title
                    );
                    let sum = text::value_summary(&a.params, 96);
                    if !sum.is_empty() && sum != "{}" {
                        t.push('\n');
                        t.push_str(&sum);
                    }
                    t.push_str("\nDouble-click to edit it, drag to move it");
                    self.tip = t;
                }
            } else if lod < 2
                && let Some(i) = self.hit_node(sc, cx.view, mouse)
                && self.hit_pin(sc, cx.view, i, mouse, grab).is_none()
                && let Some(n) = sc.nodes.get(i)
            {
                let mut t = format!("{}\n{}", n.title, n.id);
                if !n.summary.is_empty() {
                    t.push('\n');
                    t.push_str(&n.summary);
                }
                self.tip = t;
            }
        }
    }

    fn end_drag(&mut self, cx: &mut CanvasCx<'_>) {
        let Some(d) = self.drag.take() else { return };
        if !d.moved {
            return;
        }
        // one undo step: the first move is the step, the rest ride on it
        let mut at = Vec::with_capacity(d.nodes.len());
        for (k, (i, id, start)) in d.nodes.iter().enumerate() {
            let p = [layout::snap(start[0] + d.delta[0]), layout::snap(start[1] + d.delta[1])];
            at.push((*i, p));
            cx.ops.push(Op::Doc(DocOp::SetPos { id: id.to_string(), x: p[0], y: p[1], undo: k == 0 }));
        }
        self.settle = Some(Settle { from_edit: cx.scene.map_or(cx.edit, |s| s.edit), at });
    }

    fn end_link(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, mouse: Pos2, grab: f32) {
        let Some((from, pin)) = self.linking.take() else { return };
        self.status.clear();
        let mut node = self.hit_node(sc, cx.view, mouse);
        match node {
            Some(n) => {
                // a refused drop says why
                if let Some(t) = sc.nodes.get(n)
                    && let Err(why) = doc::can_link(cx.catalog, cx.graph, &from, &t.id)
                {
                    self.set_status(ctx, &why);
                    return;
                }
            }
            None => {
                // a drop beside a box still means that box: its input sits on the edge
                node = self.hit_node_near(sc, cx.view, mouse, sc.node_of(&from), grab);
                if let Some(n) = node
                    && let Some(t) = sc.nodes.get(n)
                    && doc::can_link(cx.catalog, cx.graph, &from, &t.id).is_err()
                {
                    node = None;
                }
            }
        }
        match node.and_then(|n| sc.nodes.get(n)) {
            Some(t) => cx.ops.push(Op::Doc(DocOp::Connect {
                from: from.to_string(),
                pin: pin.to_string(),
                to: t.id.to_string(),
            })),
            None if self.hovered => {
                // dropped on empty space: a node made there is wired from the pin
                let at = self.to_world(cx.view, mouse);
                self.open_menu(ctx, Menu::AddNode { at, link: Some((from, pin)) }, mouse);
            }
            None => {}
        }
    }

    fn open_menu(&mut self, ctx: &egui::Context, m: Menu, at: Pos2) {
        let picker = matches!(m, Menu::AddNode { .. } | Menu::AddAction { .. });
        self.menu = Some(m);
        self.menu_pos = at;
        if picker {
            self.filter.clear();
            self.filter_sel = 0;
            self.filter_focus = true;
        }
        egui::Popup::open_id(ctx, self.id.with("menu"));
    }

    /// A menu that asks for another one (Add Action...): it goes up after this one closed.
    fn chain_menu(&mut self, m: Menu, at: Pos2) {
        self.next_menu = Some((m, at));
    }

    fn open_context_menu(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        lod: u8,
    ) {
        if let Some(i) = self.hit_node(sc, cx.view, mouse) {
            let Some(n) = sc.nodes.get(i) else { return };
            if !cx.view.is_selected(&n.id) {
                cx.view.select_only(&n.id);
            }
            if let Some(hit) = self.hit_action(sc, cx.view, i, mouse, lod).filter(|h| !h.more)
                && let Some(row) = n.action_at(hit.row)
            {
                self.act_drag = None;
                cx.view.slot = format!("{}:{}", slot_name(row.exit), row.index);
                self.open_menu(
                    ctx,
                    Menu::Action { node: n.id.clone(), exit: row.exit, index: row.index },
                    mouse,
                );
            } else {
                self.open_menu(ctx, Menu::Node(n.id.clone()), mouse);
            }
            return;
        }
        if let Some(l) = self.hit_link(sc, cx.view, mouse).and_then(|i| sc.links.get(i)) {
            let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                return;
            };
            self.open_menu(
                ctx,
                Menu::Link { from: f.id.clone(), pin: l.pin.clone(), to: t.id.clone() },
                mouse,
            );
            return;
        }
        let at = self.to_world(cx.view, mouse);
        self.open_menu(ctx, Menu::AddNode { at, link: None }, mouse);
    }

    /// The add-action picker of a node, put up at a screen point (the Details panel's Add Action).
    pub fn request_add_action(&mut self, ctx: &egui::Context, node: &str, exit: bool, at: Pos2) {
        self.open_menu(ctx, Menu::AddAction { node: SmolStr::new(node), exit }, at);
    }

    pub fn delete_hot_link(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>) {
        let Some(l) = self.hot_link.and_then(|i| sc.links.get(i)) else { return };
        let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else { return };
        cx.ops.push(Op::Doc(DocOp::Disconnect {
            from: f.id.to_string(),
            pin: l.pin.to_string(),
            to: t.id.to_string(),
        }));
        let msg = format!("Removed {}.{} -> {}", f.id, l.pin, t.id);
        self.set_status(ctx, &msg);
        self.hot_link = None;
    }

    // ---- the keys (the tab hands them over while the canvas is the part worked in) ----------

    /// Whether the canvas takes keys now.
    #[must_use]
    pub fn takes_keys(&self, ctx: &egui::Context) -> bool {
        (self.hovered || self.focused) && !egui::Popup::is_any_open(ctx) && !ctx.egui_wants_keyboard_input()
    }

    /// A key of the canvas while it has the keys; `true` when taken. Esc drops a gesture under
    /// way; every other key of the canvas is an action of the graph (`actions.rs`).
    pub fn key(
        &mut self,
        _ctx: &egui::Context,
        _sc: Option<&Scene>,
        _cx: &mut CanvasCx<'_>,
        chord: &gz_app::Chord,
    ) -> bool {
        if chord.key != Key::Escape || chord.ctrl || chord.shift || chord.alt {
            return false;
        }
        let busy = self.linking.is_some() || self.marquee.is_some() || self.act_drag.is_some();
        self.linking = None;
        self.marquee = None;
        // drops the action back where it came from: the move only happens on release
        self.act_drag = None;
        busy
    }

    pub fn select_all(&self, cx: &mut CanvasCx<'_>) {
        cx.view.selection = cx.graph.nodes.iter().map(|n| n.id.clone()).collect();
    }

    pub fn delete_selection(&self, cx: &mut CanvasCx<'_>) {
        if !cx.view.selection.is_empty() {
            cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Delete)));
        }
    }

    /// Ctrl+B: bookmarks the selection, or takes the bookmarks off when all of it has one.
    pub fn toggle_bookmarks(&mut self, ctx: &egui::Context, cx: &mut CanvasCx<'_>) {
        if cx.view.selection.is_empty() {
            self.set_status(ctx, "Select nodes to bookmark them");
            return;
        }
        let all = cx.view.selection.iter().all(|s| cx.bookmarks.iter().any(|b| b == s));
        for s in &cx.view.selection {
            cx.ops.push(Op::Doc(DocOp::View(ViewOp::Bookmark { node: s.to_string(), on: !all })));
        }
        self.set_status(ctx, if all { "Bookmarks removed" } else { "Selection bookmarked" });
    }

    /// The selected nodes as a Lua fragment the clipboard carries (`CopySelection`).
    #[must_use]
    pub fn copy_text(graph: &Graph, selection: &[SmolStr]) -> Option<String> {
        if selection.is_empty() {
            return None;
        }
        let mut text_ = String::from("return {\n  nodes = {\n");
        for id in selection {
            let Some(n) = graph.find_node(id) else { continue };
            text_.push_str("    ");
            text_.push_str(&asset::write_node(n, 4));
            text_.push_str(",\n");
        }
        text_.push_str("  },\n}\n");
        Some(text_)
    }

    /// Pastes a fragment at the cursor (or the view's centre): fresh ids, edges kept inside the
    /// fragment, one undo step, the pasted nodes selected (`PasteClipboard`).
    pub fn paste(&mut self, ctx: &egui::Context, cx: &mut CanvasCx<'_>, clip: &str, at_mouse: Option<Pos2>) {
        let v = match gz_luadata::parse(clip) {
            Ok(v) => v,
            Err(e) => {
                self.set_status(ctx, &format!("Paste: {}", e.message));
                return;
            }
        };
        let Some(nodes) = v.get("nodes").filter(|n| n.is_table()) else {
            self.set_status(ctx, "Paste: the clipboard holds no nodes table");
            return;
        };
        let mut shape = Vec::new();
        let mut src: Vec<Node> =
            nodes.arr().iter().filter_map(|nv| asset::node_from_value(cx.catalog, nv, &mut shape)).collect();
        if src.is_empty() {
            return;
        }
        let anchor = match at_mouse {
            Some(m) if self.hovered => self.to_world(cx.view, m),
            _ => cx.view.center,
        };
        // fresh ids against the document and the fragment
        let mut remap: Vec<(SmolStr, SmolStr)> = Vec::with_capacity(src.len());
        for n in &mut src {
            let base = if n.id.is_empty() { "node".to_owned() } else { n.id.to_string() };
            let mut nid = base.clone();
            let mut k = 2;
            while cx.graph.find_node(&nid).is_some() || remap.iter().any(|(_, t)| t == nid.as_str()) {
                nid = format!("{base}{k}");
                k += 1;
            }
            remap.push((n.id.clone(), SmolStr::new(&nid)));
            n.id = SmolStr::new(&nid);
        }
        let shift = src
            .first()
            .filter(|n| n.has_pos)
            .map_or([0.0, 0.0], |f| [anchor[0] - f.pos[0], anchor[1] - f.pos[1]]);
        let mut stack_y = anchor[1];
        let mut ops: Vec<Value> = Vec::with_capacity(src.len());
        let mut made: Vec<SmolStr> = Vec::with_capacity(src.len());
        for mut n in src {
            for pin in &mut n.out {
                pin.targets = pin
                    .targets
                    .iter()
                    .filter_map(|t| remap.iter().find(|(o, _)| o == t).map(|(_, nn)| nn.clone()))
                    .collect();
            }
            n.out.retain(|p| !p.targets.is_empty());
            if n.has_pos {
                n.pos = [layout::snap(n.pos[0] + shift[0]), layout::snap(n.pos[1] + shift[1])];
            } else {
                n.has_pos = true;
                n.pos = [layout::snap(anchor[0]), layout::snap(stack_y)];
                stack_y += layout::node_size(cx.catalog, &n)[1] + layout::GAP_Y;
            }
            made.push(n.id.clone());
            ops.push(op_table(&[("op", Value::from("add_node")), ("node", asset::node_to_value(&n))]));
        }
        cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
        cx.view.selection = made;
        cx.view.slot.clear();
    }

    // ---- the menus ----------------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn draw_menus(&mut self, ui: &mut Ui, sc: &Scene, cx: &mut CanvasCx<'_>) {
        let ctx = ui.ctx().clone();
        let popup_id = self.id.with("menu");
        if !egui::Popup::is_id_open(&ctx, popup_id) {
            self.menu = None;
            if let Some((m, at)) = self.next_menu.take() {
                self.open_menu(&ctx, m, at);
            }
            return;
        }
        let Some(menu) = self.menu.clone() else {
            egui::Popup::close_id(&ctx, popup_id);
            return;
        };
        let anchor = Rect::from_min_size(self.menu_pos, Vec2::ZERO);
        let behaviour = cx.graph.is_behaviour();
        let mut close = false;
        menu_popup(&ctx, popup_id, anchor, ui.layer_id(), |ui| match &menu {
            Menu::Node(id) => {
                let item = |ui: &mut Ui, label: &str, keys: &str, enabled: bool| {
                    menu_item(ui, label, MenuItem { keys, enabled, ..MenuItem::default() })
                };
                if item(ui, "Rename", "F2", true) {
                    self.want_rename = true;
                }
                let all = !cx.view.selection.is_empty()
                    && cx.view.selection.iter().all(|s| cx.bookmarks.iter().any(|b| b == s));
                if item(ui, if all { "Remove Bookmarks" } else { "Bookmark" }, "Ctrl+B", true) {
                    self.toggle_bookmarks(&ctx, cx);
                }
                if item(ui, "Duplicate", "Ctrl+D", true) {
                    cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Duplicate)));
                }
                if item(ui, "Copy", "Ctrl+C", true)
                    && let Some(t) = Self::copy_text(cx.graph, &cx.view.selection)
                {
                    ctx.copy_text(t);
                }
                ui.separator();
                let trigger = cx.graph.find_node(id).is_some_and(|n| cx.catalog.is_trigger(&n.kind));
                let at = self.menu_pos;
                if item(ui, "Add Action on Enter...", "", !trigger) {
                    self.chain_menu(Menu::AddAction { node: id.clone(), exit: false }, at);
                }
                if item(ui, "Add Action on Exit...", "", !trigger) {
                    self.chain_menu(Menu::AddAction { node: id.clone(), exit: true }, at);
                }
                if item(ui, "Connect to Nearest Free Pin", "Q", cx.view.selection.len() == 1) {
                    cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::ConnectNearest)));
                }
                if item(ui, "Bypass", "Alt+D", true) {
                    cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Bypass)));
                }
                if item(ui, "Disconnect Outputs", "", true) {
                    cx.ops.push(Op::Doc(DocOp::Disconnect {
                        from: id.to_string(),
                        pin: String::new(),
                        to: String::new(),
                    }));
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Delete",
                    MenuItem { icon: Icon::Delete, keys: "Del", danger: true, ..MenuItem::default() },
                ) {
                    self.delete_selection(cx);
                }
            }
            Menu::Action { node, exit, index } => {
                let gn = cx.graph.find_node(node);
                let list_len = gn.map_or(0, |n| if *exit { n.on_exit.len() } else { n.on_enter.len() });
                let other_len = gn.map_or(0, |n| if *exit { n.on_enter.len() } else { n.on_exit.len() });
                let valid = *index < list_len;
                let enabled = |on: bool| MenuItem { enabled: on, ..MenuItem::default() };
                if menu_item(ui, "Duplicate Action", enabled(valid))
                    && let Some(a) =
                        gn.and_then(|n| (if *exit { &n.on_exit } else { &n.on_enter }).get(*index))
                {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("add_action")),
                        ("id", Value::from(node.as_str())),
                        ("slot", Value::from(slot_name(*exit))),
                        ("index", Value::from((*index + 2) as f64)),
                        ("action", asset::action_to_value(a)),
                    ]))));
                }
                if menu_item(ui, "Move Up", enabled(valid && *index > 0)) {
                    move_action(cx, node, *exit, *index, node, *exit, index - 1);
                }
                if menu_item(ui, "Move Down", enabled(valid && index + 1 < list_len)) {
                    move_action(cx, node, *exit, *index, node, *exit, index + 2);
                }
                if menu_item(ui, if *exit { "Move to On Enter" } else { "Move to On Exit" }, enabled(valid)) {
                    move_action(cx, node, *exit, *index, node, !*exit, other_len);
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Remove Action",
                    MenuItem { enabled: valid, danger: true, ..MenuItem::default() },
                ) {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("remove_action")),
                        ("id", Value::from(node.as_str())),
                        ("slot", Value::from(slot_name(*exit))),
                        ("index", Value::from((*index + 1) as f64)),
                    ]))));
                    cx.view.slot.clear();
                }
            }
            Menu::Link { from, pin, to } => {
                gz_ui::text::label(
                    ui,
                    &format!("{from}.{pin} -> {to}"),
                    Font::Small,
                    palette(&ctx).text_muted,
                    gz_ui::Clip::Middle,
                );
                if menu_item(ui, "Remove Link", MenuItem { danger: true, ..MenuItem::default() }) {
                    cx.ops.push(Op::Doc(DocOp::Disconnect {
                        from: from.to_string(),
                        pin: pin.to_string(),
                        to: to.to_string(),
                    }));
                }
            }
            Menu::AddNode { at, link } => {
                if let Some(kind) = local::pick_kind(
                    ui,
                    cx.catalog,
                    USE_TRIGGER | USE_MAIN,
                    behaviour,
                    "Search kinds",
                    &mut self.filter,
                    &mut self.filter_sel,
                    &mut self.filter_focus,
                ) {
                    create_node(cx, &kind, *at, link.as_ref());
                    close = true;
                }
            }
            Menu::AddAction { node, exit } => {
                if let Some(kind) = local::pick_kind(
                    ui,
                    cx.catalog,
                    USE_EXTRA,
                    behaviour,
                    "Search actions",
                    &mut self.filter,
                    &mut self.filter_sel,
                    &mut self.filter_focus,
                ) {
                    let mut a = Value::table();
                    a.set("kind", kind.as_str());
                    let len = cx
                        .graph
                        .find_node(node)
                        .map_or(0, |n| if *exit { n.on_exit.len() } else { n.on_enter.len() });
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("add_action")),
                        ("id", Value::from(node.as_str())),
                        ("slot", Value::from(slot_name(*exit))),
                        ("action", a),
                    ]))));
                    cx.view.select_only(node);
                    cx.view.slot = format!("{}:{len}", slot_name(*exit));
                    self.want_focus_action = true;
                    close = true;
                }
            }
        });
        if close {
            egui::Popup::close_id(&ctx, popup_id);
        }
        let _ = sc;
    }
}

/// Moves one action: within its slot, to the other slot or to another node - one undo step, and
/// the Details panel follows it to where it landed (`MoveAction`).
fn move_action(
    cx: &mut CanvasCx<'_>,
    from: &str,
    from_exit: bool,
    index: usize,
    to: &str,
    to_exit: bool,
    at: usize,
) {
    // dropping an action on its own place is not a move
    if from == to && from_exit == to_exit && (at == index || at == index + 1) {
        return;
    }
    let mut pos = at;
    if from == to && from_exit == to_exit && pos > index {
        pos -= 1;
    }
    let dst_len =
        cx.graph.find_node(to).map_or(0, |n| if to_exit { n.on_exit.len() } else { n.on_enter.len() });
    let dst_len = if from == to && from_exit == to_exit { dst_len.saturating_sub(1) } else { dst_len };
    let pos = pos.min(dst_len);
    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
        ("op", Value::from("move_action")),
        ("id", Value::from(from)),
        ("slot", Value::from(slot_name(from_exit))),
        ("index", Value::from((index + 1) as f64)),
        ("to_id", Value::from(to)),
        ("to_slot", Value::from(slot_name(to_exit))),
        ("to_index", Value::from((pos + 1) as f64)),
    ]))));
    cx.view.select_only(to);
    cx.view.slot = format!("{}:{pos}", slot_name(to_exit));
}

/// A node of a kind at a world point, wired from a pin when it was asked for by a link: one undo
/// step (`CreateNode`).
fn create_node(cx: &mut CanvasCx<'_>, kind: &str, at: [f32; 2], link: Option<&(SmolStr, SmolStr)>) {
    let last = kind.rsplit('.').next().unwrap_or(kind);
    let base = if last == "start" || last == "when" { "trigger" } else { last };
    let id = cx.graph.free_node_id(base);
    if id.is_empty() {
        return;
    }
    let mut n = Node::new(&id, kind);
    n.params = Value::table();
    n.pos = [layout::snap(at[0]), layout::snap(at[1])];
    n.has_pos = true;
    let mut ops = vec![op_table(&[("op", Value::from("add_node")), ("node", asset::node_to_value(&n))])];
    if let Some((from, pin)) = link {
        ops.push(op_table(&[
            ("op", Value::from("connect")),
            ("from", Value::from(from.as_str())),
            ("pin", Value::from(pin.as_str())),
            ("to", Value::from(id.as_str())),
        ]));
    }
    cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
    cx.view.select_only(&id);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use egui::{Context, Event, Modifiers, MouseWheelUnit, Pos2, RawInput, Rect, pos2, vec2};
    use gz_nq::{Catalog, Node, asset::Graph};

    use super::{Canvas, CanvasCx, ViewMirror};
    use crate::derived::DocData;
    use crate::scene::{Scene, SceneInput};

    /// The wheel over a panel the tab draws over the canvas (the Keyboard Shortcuts card) is that
    /// panel's: the graph does not zoom under it, and it does beside it.
    #[test]
    fn the_wheel_over_a_panel_on_the_canvas_does_not_zoom_the_graph() {
        let ctx = Context::default();
        gz_ui::font::install(&ctx, gz_ui::font::FontFiles::default());
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.has_pos = true;
        g.nodes.push_back(s);
        let cat = Catalog::empty();
        let p = gz_ui::Palette::new(gz_ui::Accent::default());
        let scene = Scene::build(&SceneInput { graph: &g, catalog: &cat, edit: 1, palette: &p });
        let data = DocData::default();
        let mut view = ViewMirror::default();
        let mut canvas = Canvas::new("test", false);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
        let card = Rect::from_min_max(pos2(600.0, 8.0), pos2(992.0, 692.0));
        let mut t = 0.0;
        let mut wheel_at = |at: Pos2, view: &mut ViewMirror, canvas: &mut Canvas| {
            let wheel = Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
                source: egui::MouseWheelSource::Wheel,
            };
            for events in [vec![Event::PointerMoved(at)], vec![wheel], vec![]] {
                t += 0.05;
                let mut ops = Vec::new();
                let mut out = ctx.run_ui(
                    RawInput { screen_rect: Some(rect), time: Some(t), events, ..RawInput::default() },
                    |ui| {
                        let mut cx = CanvasCx {
                            scene: Some(&scene),
                            graph: &g,
                            catalog: &cat,
                            data: &data,
                            view,
                            bookmarks: &[],
                            minimap: false,
                            ops: &mut ops,
                            edit: 1,
                            doc_status: None,
                            covered: &[card],
                        };
                        canvas.draw(ui, rect, &mut cx);
                    },
                );
                out.textures_delta.clear();
            }
        };
        let zoom = view.zoom;
        wheel_at(pos2(800.0, 300.0), &mut view, &mut canvas);
        assert_eq!(view.zoom, zoom, "the graph zoomed under the card");
        wheel_at(pos2(300.0, 300.0), &mut view, &mut canvas);
        assert_ne!(view.zoom, zoom, "the graph did not zoom beside the card");
    }
}
