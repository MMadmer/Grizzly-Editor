//! The graph canvas: an infinite world painted by hand. Right or middle drag pans, the wheel
//! zooms at the cursor over the levels of `gz_nq::doc::ZOOM` (5% to 200%), the flow runs left to
//! right or top to bottom (the graph's `canvas.flow`). A node is drawn from the scene's box (the
//! one `gz_nq::layout` makes), only when the view touches its grid cell; a wire is the polyline
//! `gz_nq::wires` builds, drawn and hit along the same points.
//!
//! **Semantic zoom.** Text is never drawn smaller than 11 px (its line box, as UI_DESIGN section
//! 2 names sizes): while a body line would be smaller, a node shows its title alone, drawn at a
//! fixed size over its whole box; a node too small even for that is a block of its family's colour
//! with its title on hover.
//!
//! Every edit is a `gz_nq` document command the tab sends, or an action of the group Graph; what
//! the canvas keeps is view state: the gesture under way, what is under the cursor, the status.

use std::sync::Arc;

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use gz_editor::Editor;
use gz_luadata::Value;
use gz_nq::asset::{self, Graph, Node};
use gz_nq::canvas::Flow;
use gz_nq::catalog::{USE_EXTRA, USE_MAIN, USE_TRIGGER};
use gz_nq::doc::{self, CanvasOp, DocOp, ZOOM, ZOOM_DEFAULT, zoom_of};
use gz_nq::layout::{self, ACTION_ROW, COMMENT, GRID, HEADER, INSET, LINE, PAD};
use gz_nq::{Catalog, wires};
use gz_ui::menu::{MenuItem, menu_item, menu_popup};
use gz_ui::{Font, Icon, Palette, PickItem, PickList, icons, palette};
use smol_str::SmolStr;

use crate::derived::DocData;
use crate::look::{self, Family};
use crate::scene::{self, MINIMAP_H, MINIMAP_W, PinRole, Scene, SceneNode};

/// Line boxes of the canvas's text at 100%, world pt.
const TITLE_LINE: f32 = 16.0;
const BODY_LINE: f32 = 15.0;
/// No text is drawn with a line box under this many px.
pub const MIN_TEXT_PX: f32 = 11.0;
/// A node's title while the body is too small to read: this many px, whatever the zoom.
pub const FIXED_TITLE_PX: f32 = 13.0;
/// A node narrower than this on screen is a block.
const HEADER_MIN_PX: f32 = 48.0;
const PIN_R: f32 = 4.5;
const PIN_GRAB_PAD: f32 = 8.0;
const PIN_GRAB_MIN: f32 = 12.0;
const PIN_GRAB_MAX: f32 = 22.0;
const LINK_GRAB: f32 = 10.0;
const STATUS_SECONDS: f64 = 2.5;
/// A view at rest this long after a wheel step reaches the document (and its history).
const WHEEL_SETTLE: f64 = 0.35;
/// How close an edge snaps to a neighbour's while dragging, px.
const GUIDE_PX: f32 = 6.0;
/// A frame's title strip and its resize handle, world pt.
const FRAME_TITLE: f32 = 32.0;
const FRAME_HANDLE: f32 = 14.0;
/// How fast the flow bubbles travel, world pt a second.
const BUBBLE_SPEED: f32 = 160.0;

/// How much of a node the zoom lets the canvas draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Everything: lines, actions, the comment, the id.
    Full,
    /// The title alone, at a fixed size over the whole box.
    Header,
    /// A block of the family's colour; the title on hover.
    Block,
}

impl Detail {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Detail::Full => "full",
            Detail::Header => "header",
            Detail::Block => "block",
        }
    }
}

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

/// The preferences the canvas's gestures read.
#[derive(Clone, Copy, Debug)]
pub struct CanvasPrefs {
    pub snap: bool,
    pub guides: bool,
}

impl Default for CanvasPrefs {
    fn default() -> Self {
        Self { snap: true, guides: true }
    }
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
    /// The selected reroutes that go along: wire (a link index of the scene the drag started
    /// over), its place among the wire's points, start position; and the same reroutes by their
    /// wire and point, to find them again in a scene that came meanwhile.
    knots: Vec<(usize, usize, [f32; 2])>,
    knot_refs: Vec<KnotRef>,
    /// The scene's content serial when the drag started.
    edit: u64,
    /// The node under the pointer: the one the grid and the guides place (none when a reroute is).
    anchor: usize,
    start: Pos2,
    moved: bool,
    delta: [f32; 2],
    /// The wire a lone, unwired node is held over: it goes on it when it is let go.
    on_wire: Option<usize>,
    /// A plain press on one of several selected things: let go without travel, it alone stays
    /// selected (a node by its id, or a reroute).
    single: Option<Single>,
}

#[derive(Clone, Debug)]
enum Single {
    Node(SmolStr),
    Knot(KnotRef),
}

/// Positions committed by a drag, drawn until the scene that holds them arrives.
#[derive(Clone, Debug)]
struct Settle {
    from_edit: u64,
    at: Vec<(usize, [f32; 2])>,
    /// The reroutes moved: wire, place, position.
    knots: Vec<(usize, usize, [f32; 2])>,
}

#[derive(Clone, Debug)]
struct ActDrag {
    node: SmolStr,
    exit: bool,
    index: usize,
    start: Pos2,
    dragging: bool,
}

/// What a double click asks for once the button is let go: a field focused at the press would
/// lose its focus at the release, which egui takes for a click elsewhere.
#[derive(Clone, Debug)]
enum OnRelease {
    /// The Details panel focuses the node's id.
    Rename,
    /// A frame's title is edited in place.
    FrameTitle(usize, String),
}

/// A frame being moved (with the nodes inside it) or resized.
#[derive(Clone, Debug)]
struct FrameDrag {
    index: usize,
    start: Pos2,
    resize: bool,
    delta: [f32; 2],
    inside: Vec<usize>,
}

/// A selected reroute: its wire by the ends and the pin, its place among the wire's points and
/// the point itself. A wire's index in the scene changes with every edit, and a place among the
/// points with every reroute put in or taken out before it; the point says which one it was.
#[derive(Clone, Debug, PartialEq)]
struct KnotRef {
    from: SmolStr,
    pin: SmolStr,
    to: SmolStr,
    index: usize,
    at: [f32; 2],
}

impl KnotRef {
    fn same_wire(&self, o: &KnotRef) -> bool {
        self.from == o.from && self.pin == o.pin && self.to == o.to
    }

    /// The same reroute: one wire, one point.
    fn same(&self, o: &KnotRef) -> bool {
        self.same_wire(o) && near_point(self.at, o.at)
    }
}

fn near_point(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
}

/// Where a wire being drawn starts: an output pin, or an input it comes into.
#[derive(Clone, Debug, PartialEq)]
enum Linking {
    From { node: SmolStr, pin: SmolStr },
    Into { node: SmolStr },
}

/// A wire whose end is being moved elsewhere (Alt+drag): which end.
#[derive(Clone, Debug)]
struct Rewire {
    from: SmolStr,
    pin: SmolStr,
    to: SmolStr,
    /// The input end moves (else the pin's).
    target: bool,
}

/// What the picker is for.
#[derive(Clone, Debug, PartialEq)]
enum PickFor {
    Canvas { at: [f32; 2] },
    FromPin { from: SmolStr, pin: SmolStr, at: [f32; 2] },
    IntoInput { to: SmolStr, at: [f32; 2] },
    Action { node: SmolStr, exit: bool },
    OnWire { from: SmolStr, pin: SmolStr, to: SmolStr, at: [f32; 2] },
    Replace { node: SmolStr },
}

/// The popup of the canvas that is up.
#[derive(Clone, Debug, PartialEq)]
enum Menu {
    Node(SmolStr),
    Action { node: SmolStr, exit: bool, index: usize },
    Wire { from: SmolStr, pin: SmolStr, to: SmolStr, at: [f32; 2] },
    Canvas { at: [f32; 2] },
    Frame(usize),
    Knot { link: usize, index: usize },
    Picker(PickFor),
}

impl Menu {
    fn name(&self) -> &'static str {
        match self {
            Menu::Node(_) | Menu::Action { .. } => "node",
            Menu::Wire { .. } | Menu::Knot { .. } => "wire",
            Menu::Canvas { .. } => "canvas",
            Menu::Frame(_) => "frame",
            Menu::Picker(_) => "picker",
        }
    }
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
    minimap_k: f32,
    /// The wires downstream of the selection (Flow Bubbles), for a revision and a selection.
    flowing: Option<(u64, Vec<SmolStr>, Vec<bool>)>,
    pub beyond: [usize; 4],
    beyond_near: [Option<u32>; 4],
    beyond_hot: Option<usize>,
    pan: Option<(Pos2, bool)>,
    middle_pan: bool,
    drag: Option<Drag>,
    settle: Option<Settle>,
    marquee: Option<Pos2>,
    linking: Option<Linking>,
    /// Where the wire being drawn was pressed: let go there, it was a click on the node.
    link_press: Pos2,
    rewire: Option<Rewire>,
    /// An Alt press on a wire: a click removes it, a drag moves its nearer end.
    alt_press: Option<(usize, Pos2)>,
    act_drag: Option<ActDrag>,
    frame_drag: Option<FrameDrag>,
    /// A frame's place committed by a move or a resize, drawn until the scene that holds it
    /// arrives: the edit it was made over, the frame, its rectangle.
    frame_settle: Option<(u64, usize, [f32; 4])>,
    /// The selected reroutes (view state of the canvas alone: the document selects nodes).
    knots: Vec<KnotRef>,
    /// `knots` in this frame's scene: wire (link index), place.
    knot_hits: Vec<(usize, usize)>,
    /// The node selection as the last frame left it: one replaced from outside the canvas (Find,
    /// the Problems drawer, a command, undo) drops the selected reroutes.
    seen_selection: Vec<SmolStr>,
    /// A gesture of this frame chose the reroutes with the nodes: a change of the node selection
    /// does not drop them.
    knots_kept: bool,
    /// The last press of the left button, for a double click told at the press: egui tells one
    /// at the release, after the press already started a gesture.
    last_press: Option<(f64, Pos2)>,
    /// Ctrl + right drag: the cut line's start.
    cut: Option<Pos2>,
    hot_node: Option<usize>,
    /// The pin of `hot_node` under the cursor; `None` with a node is its input.
    hot_pin: Option<usize>,
    hot_link: Option<usize>,
    hot_action: Option<ActHit>,
    hot_frame: Option<(usize, bool)>,
    hot_knot: Option<(usize, usize)>,
    guides: Vec<[Pos2; 2]>,
    status: gz_ui::Tx<'static>,
    status_at: f64,
    tip: String,
    menu: Option<Menu>,
    menu_pos: Pos2,
    next_menu: Option<(Menu, Pos2)>,
    /// The picker's rows, made once when it opens.
    pick_items: Vec<PickItem>,
    /// A frame's title being edited in place.
    frame_title: Option<(usize, String, bool)>,
    on_release: Option<OnRelease>,
    want_frame_all: bool,
    want_frame_sel: bool,
    /// A zoom asked for explicitly: applied after a pending frame picked its own.
    pub want_zoom: Option<i32>,
    /// F2 or a rename from the menu: the Details panel focuses the id.
    pub want_rename: bool,
    /// A double click on an action row: the Details panel focuses its kind.
    pub want_focus_action: bool,
    /// Paste from the empty canvas's menu, at this world point: the tab reads the clipboard.
    pub want_paste: Option<[f32; 2]>,
    /// The view moved by a gesture that has not reached the document yet.
    pub view_moving: bool,
    /// What the last frame drew of the nodes it showed: the least title px and the detail.
    pub drawn_detail: Detail,
    pub drawn_title_px: f32,
    last_wheel: f64,
    last_mouse: Pos2,
    stamp: Vec<u32>,
    serial: u32,
    scratch: Vec<u32>,
}

/// What a frame of the canvas reads and where it writes.
pub struct CanvasCx<'a> {
    pub scene: Option<&'a Scene>,
    pub graph: &'a Graph,
    pub catalog: &'a Catalog,
    pub data: &'a DocData,
    pub view: &'a mut ViewMirror,
    pub minimap: bool,
    pub ops: &'a mut Vec<Op>,
    /// Actions of the group Graph the canvas's menus picked: the tab runs them.
    pub actions: &'a mut Vec<&'static str>,
    /// The editor whose actions the menus show; none in a test.
    pub editor: Option<&'a Arc<Editor>>,
    pub prefs: CanvasPrefs,
    /// The document's content serial now (a drag's commit waits for a scene past it).
    pub edit: u64,
    /// The status the document's last canvas command left, when it is new.
    pub doc_status: Option<String>,
    /// What the tab draws over the canvas (the Keyboard Shortcuts card, the rail's Graph panel), as the
    /// last frame laid it out: the pointer over one of them is not over the canvas - a click, the wheel
    /// and a hover there are that panel's.
    pub covered: &'a [Rect],
    /// Where the picker keeps its Recent and Favourites.
    pub pick_memory: &'a str,
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

fn round(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// One line of canvas text cut to `avail`, painted with its left edge at `at.x` and its middle
/// at `at.y`; the width drawn. The cut is the kit's cached one: worked out once per text, size
/// and width, not per frame. Canvas text is clipped by its node on purpose, so it is not
/// reported to the audit. `text_` is as the window shows it: a word of the editor's already in
/// its language.
fn paint_fit(painter: &Painter, font: &FontId, at: Pos2, color: Color32, text_: &str, avail: f32) -> f32 {
    if text_.is_empty() || avail <= font.size * 0.5 {
        return 0.0;
    }
    let fitted = gz_ui::text::fit(painter.ctx(), font, gz_ui::Tx::raw(text_), avail, gz_ui::Clip::End);
    painter.text(at, Align2::LEFT_CENTER, &*fitted.text, font.clone(), color).width()
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

fn point(p: [f32; 2]) -> Value {
    Value::from(vec![Value::from(f64::from(p[0])), Value::from(f64::from(p[1]))])
}

/// The segment `a`-`b` crosses `c`-`d`.
fn crosses(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    let cross =
        |o: [f32; 2], p: [f32; 2], q: [f32; 2]| (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
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
            minimap_k: 1.0,
            flowing: None,
            beyond: [0; 4],
            beyond_near: [None; 4],
            beyond_hot: None,
            pan: None,
            middle_pan: false,
            drag: None,
            settle: None,
            marquee: None,
            linking: None,
            link_press: Pos2::ZERO,
            rewire: None,
            alt_press: None,
            act_drag: None,
            frame_drag: None,
            frame_settle: None,
            knots: Vec::new(),
            knot_hits: Vec::new(),
            seen_selection: Vec::new(),
            knots_kept: false,
            last_press: None,
            cut: None,
            hot_node: None,
            hot_pin: None,
            hot_link: None,
            hot_action: None,
            hot_frame: None,
            hot_knot: None,
            guides: Vec::new(),
            status: gz_ui::Tx::EMPTY,
            status_at: 0.0,
            tip: String::new(),
            menu: None,
            menu_pos: Pos2::ZERO,
            next_menu: None,
            pick_items: Vec::new(),
            frame_title: None,
            on_release: None,
            want_frame_all: frame_on_open,
            want_frame_sel: false,
            want_zoom: None,
            want_rename: false,
            want_focus_action: false,
            want_paste: None,
            view_moving: false,
            drawn_detail: Detail::Full,
            drawn_title_px: 0.0,
            last_wheel: f64::MIN,
            last_mouse: Pos2::ZERO,
            stamp: Vec::new(),
            serial: 0,
            scratch: Vec::new(),
        }
    }

    // ---- the view ---------------------------------------------------------------------------

    /// Points per world unit: the zoom level's factor (egui's unit is the point).
    #[must_use]
    pub fn scale(view: &ViewMirror) -> f32 {
        zoom_of(view.zoom)
    }

    /// The whole of a node is drawn at this zoom: a body line is at least 11 px.
    #[must_use]
    pub fn full(view: &ViewMirror, ppp: f32) -> bool {
        BODY_LINE * zoom_of(view.zoom) * ppp >= MIN_TEXT_PX
    }

    /// How much of a node of this width is drawn.
    #[must_use]
    pub fn detail(view: &ViewMirror, ppp: f32, width: f32) -> Detail {
        if Self::full(view, ppp) {
            Detail::Full
        } else if width * zoom_of(view.zoom) * ppp >= HEADER_MIN_PX {
            Detail::Header
        } else {
            Detail::Block
        }
    }

    /// The px of a title's line box at this zoom: its size, or the fixed size of the semantic
    /// zoom.
    #[must_use]
    pub fn title_px(view: &ViewMirror, ppp: f32) -> f32 {
        if Self::full(view, ppp) { round(TITLE_LINE * zoom_of(view.zoom) * ppp) } else { FIXED_TITLE_PX }
    }

    /// A face at a line box of `line_px` px, as an egui font (pt, em = 0.75 of the line).
    fn font_px(face: Font, line_px: f32, ppp: f32) -> FontId {
        face.at(round(line_px.max(1.0)) / ppp * 0.75)
    }

    /// A size of canvas type at this level: the line box rounded to whole px.
    fn font(face: Font, line_pt: f32, view: &ViewMirror, ppp: f32) -> FontId {
        Self::font_px(face, line_pt * zoom_of(view.zoom) * ppp, ppp)
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

    /// An explicit view (`graph.view`'s zoom or centre) wins over a pending frame.
    pub fn cancel_framing(&mut self) {
        self.want_frame_all = false;
        self.want_frame_sel = false;
    }

    /// A line the canvas says for a few seconds: the editor's own words (`Tx::ui` for a sentence
    /// the graph area wrote).
    pub fn set_status(&mut self, ctx: &egui::Context, text_: impl Into<gz_ui::Tx<'static>>) {
        self.status = text_.into();
        self.status_at = ctx.input(|i| i.time);
    }

    /// One zoom step, around a screen point when given.
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

    fn frame_all_now(&mut self, sc: &Scene, view: &mut ViewMirror) {
        let f = scene::frame_all(sc, [self.size.x, self.size.y]);
        view.zoom = f.zoom;
        view.center = f.center;
    }

    fn frame_selection_now(&mut self, sc: &Scene, view: &mut ViewMirror) {
        let mut r: Option<[f32; 4]> = None;
        for id in &view.selection {
            let Some(n) = sc.node_of(id).and_then(|i| sc.nodes.get(i)) else { continue };
            let b = n.rect();
            r = Some(r.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]));
        }
        match r {
            None => self.frame_all_now(sc, view),
            Some(r) => {
                let (z, c) = scene::frame_rect(r, [self.size.x, self.size.y]);
                view.zoom = z;
                view.center = c;
            }
        }
    }

    /// Brings the nearest node beyond an edge into view.
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

    // ---- geometry under a gesture ----------------------------------------------------------

    /// Where a node is drawn now: the scene's place, moved by a drag, a frame's move or a commit
    /// in flight.
    fn pos_of(&self, sc: &Scene, i: usize) -> [f32; 2] {
        let base = sc.nodes.get(i).map_or([0.0, 0.0], |n| n.pos);
        if let Some(d) = &self.drag
            && d.moved
            && let Some((_, _, start)) = d.nodes.iter().find(|(j, _, _)| *j == i)
        {
            return add(*start, d.delta);
        }
        if let Some(f) = &self.frame_drag
            && !f.resize
            && f.inside.contains(&i)
        {
            return add(base, f.delta);
        }
        if let Some(s) = &self.settle
            && let Some((_, p)) = s.at.iter().find(|(j, _)| *j == i)
        {
            return *p;
        }
        base
    }

    fn moving(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moved) || self.settle.is_some() || self.frame_drag.is_some()
    }

    /// Where a reroute is drawn now: the scene's place, moved by a drag or a commit in flight.
    fn knot_at(&self, sc: &Scene, li: usize, k: usize) -> [f32; 2] {
        let base = sc.links.get(li).and_then(|l| l.via.get(k)).copied().unwrap_or([0.0, 0.0]);
        // a frame being moved carries the wires it holds whole
        if let Some(f) = &self.frame_drag
            && !f.resize
            && sc
                .links
                .get(li)
                .is_some_and(|l| f.inside.contains(&(l.from as usize)) && f.inside.contains(&(l.to as usize)))
        {
            return add(base, f.delta);
        }
        if let Some(d) = &self.drag
            && d.moved
            && let Some((_, _, start)) = d.knots.iter().find(|(l, j, _)| *l == li && *j == k)
        {
            return add(*start, d.delta);
        }
        if let Some(s) = &self.settle
            && let Some((_, _, p)) = s.knots.iter().find(|(l, j, _)| *l == li && *j == k)
        {
            return *p;
        }
        base
    }

    /// A reroute of this wire is being moved (or its move is in flight).
    fn knots_moving(&self, sc: &Scene, li: usize) -> bool {
        let held = |f: &FrameDrag| {
            sc.links
                .get(li)
                .is_some_and(|l| f.inside.contains(&(l.from as usize)) && f.inside.contains(&(l.to as usize)))
        };
        self.frame_drag.as_ref().is_some_and(|f| !f.resize && held(f))
            || self.drag.as_ref().is_some_and(|d| d.moved && d.knots.iter().any(|(l, _, _)| *l == li))
            || self.settle.as_ref().is_some_and(|s| s.knots.iter().any(|(l, _, _)| *l == li))
    }

    /// The reroute is selected.
    fn knot_selected(&self, li: usize, k: usize) -> bool {
        self.knot_hits.contains(&(li, k))
    }

    /// The selected reroutes found in this frame's scene, each at its point (its place among the
    /// wire's points moves when one before it is put in or taken out); one whose wire or point is
    /// gone drops out of the selection.
    fn resolve_knots(&mut self, sc: &Scene) {
        self.knot_hits.clear();
        if self.knots.is_empty() {
            return;
        }
        let by_wire = Self::wires_by_ends(sc);
        let mut hits = Vec::with_capacity(self.knots.len());
        self.knots.retain_mut(|r| {
            let Some(&li) = by_wire.get(&(r.from.as_str(), r.pin.as_str(), r.to.as_str())) else {
                return false;
            };
            let Some(l) = sc.links.get(li) else { return false };
            let k = if l.via.get(r.index).is_some_and(|v| near_point(*v, r.at)) {
                Some(r.index)
            } else {
                l.via.iter().position(|v| near_point(*v, r.at))
            };
            let Some(k) = k else { return false };
            r.index = k;
            hits.push((li, k));
            true
        });
        self.knot_hits = hits;
    }

    /// The wires of a scene by their ends and pin.
    fn wires_by_ends(sc: &Scene) -> std::collections::HashMap<(&str, &str, &str), usize> {
        let mut by_wire = std::collections::HashMap::with_capacity(sc.links.len());
        for (li, l) in sc.links.iter().enumerate() {
            if let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) {
                by_wire.insert((f.id.as_str(), l.pin.as_str(), t.id.as_str()), li);
            }
        }
        by_wire
    }

    /// A reference found again in another scene: the wire's index and the point's place.
    fn find_knot(sc: &Scene, r: &KnotRef) -> Option<(usize, usize)> {
        let li = sc.links.iter().position(|l| {
            l.pin == r.pin
                && sc.nodes.get(l.from as usize).is_some_and(|n| n.id == r.from)
                && sc.nodes.get(l.to as usize).is_some_and(|n| n.id == r.to)
        })?;
        let k = sc.links.get(li)?.via.iter().position(|v| near_point(*v, r.at))?;
        Some((li, k))
    }

    /// The reference of a reroute of this frame's scene.
    fn knot_ref(sc: &Scene, li: usize, k: usize) -> Option<KnotRef> {
        let l = sc.links.get(li)?;
        let (f, t) = (sc.nodes.get(l.from as usize)?, sc.nodes.get(l.to as usize)?);
        Some(KnotRef {
            from: f.id.clone(),
            pin: l.pin.clone(),
            to: t.id.clone(),
            index: k,
            at: *l.via.get(k)?,
        })
    }

    /// A reroute taken out of its wire leaves the selection.
    fn forget_knot(&mut self, sc: &Scene, li: usize, k: usize) {
        if let Some(w) = Self::knot_ref(sc, li, k) {
            self.knots.retain(|r| !r.same(&w));
        }
    }

    /// Every reroute of a wire leaves the selection.
    fn drop_knots_of(&mut self, sc: &Scene, li: usize) {
        let Some(l) = sc.links.get(li) else { return };
        let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else { return };
        self.knots.retain(|r| !(r.from == f.id && r.pin == l.pin && r.to == t.id));
    }

    /// A selected reroute moved from `was` to `now`: its reference follows it.
    fn knot_moved(&mut self, w: &KnotRef, now: [f32; 2]) {
        for r in &mut self.knots {
            if r.same(w) {
                r.at = now;
            }
        }
    }

    /// The node selection was set from outside the canvas (a command): the reroutes it chose
    /// with it stay.
    pub fn selection_seen(&mut self, selection: &[SmolStr]) {
        self.seen_selection = selection.to_vec();
    }

    /// No reroute is selected.
    pub fn clear_knots(&mut self) {
        self.knots.clear();
        self.knot_hits.clear();
    }

    /// How many reroutes are selected.
    #[must_use]
    pub fn selected_knots(&self) -> usize {
        self.knots.len()
    }

    /// Selects every reroute of the scene (Select All), or none.
    pub fn select_all_knots(&mut self, sc: Option<&Scene>, all: bool) {
        self.knots.clear();
        if all && let Some(sc) = sc {
            for (li, l) in sc.links.iter().enumerate() {
                for k in 0..l.via.len() {
                    self.knots.extend(Self::knot_ref(sc, li, k));
                }
            }
        }
    }

    /// What Delete does to the selection when reroutes are in it: the nodes removed first, then
    /// one `route` op per wire that stays, with the points that stay - one undo step named after
    /// what goes. `None` when no reroute is selected (the document's own Delete takes the nodes).
    #[must_use]
    pub fn delete_ops(&mut self, sc: &Scene, nodes: &[SmolStr]) -> Option<Value> {
        self.resolve_knots(sc);
        if self.knot_hits.is_empty() {
            return None;
        }
        let mut ops = Vec::new();
        if !nodes.is_empty() {
            let ids: Vec<Value> = nodes.iter().map(|s| Value::from(s.as_str())).collect();
            ops.push(op_table(&[("op", Value::from("remove")), ("nodes", Value::from(ids))]));
        }
        let mut wires: Vec<usize> = self.knot_hits.iter().map(|(li, _)| *li).collect();
        wires.sort_unstable();
        wires.dedup();
        for li in wires {
            let Some(l) = sc.links.get(li) else { continue };
            let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                continue;
            };
            // a wire of a node that goes, goes with it
            if nodes.iter().any(|n| *n == f.id || *n == t.id) {
                continue;
            }
            let keep: Vec<Value> = l
                .via
                .iter()
                .enumerate()
                .filter(|(k, _)| !self.knot_hits.contains(&(li, *k)))
                .map(|(_, q)| point(*q))
                .collect();
            ops.push(op_table(&[
                ("op", Value::from("route")),
                ("from", Value::from(f.id.as_str())),
                ("pin", Value::from(l.pin.as_str())),
                ("to", Value::from(t.id.as_str())),
                ("points", Value::from(keep)),
            ]));
        }
        self.clear_knots();
        Some(Value::from(ops))
    }

    fn rect_of(&self, sc: &Scene, view: &ViewMirror, i: usize) -> Rect {
        let n = sc.nodes.get(i);
        let p = self.pos_of(sc, i);
        let size = n.map_or([0.0, 0.0], |n| n.size);
        Rect::from_min_max(self.to_screen(view, p), self.to_screen(view, add(p, size)))
    }

    fn output_pin_w(&self, sc: &Scene, i: usize, pin: usize) -> [f32; 2] {
        let Some(n) = sc.nodes.get(i) else { return [0.0, 0.0] };
        layout::output_pin(sc.flow, self.pos_of(sc, i), n.size, pin, n.pins.len())
    }

    fn input_pin_w(&self, sc: &Scene, i: usize) -> [f32; 2] {
        let Some(n) = sc.nodes.get(i) else { return [0.0, 0.0] };
        layout::input_pin(sc.flow, self.pos_of(sc, i), n.size)
    }

    fn output_pin(&self, sc: &Scene, view: &ViewMirror, i: usize, pin: usize) -> Pos2 {
        self.to_screen(view, self.output_pin_w(sc, i, pin))
    }

    fn input_pin(&self, sc: &Scene, view: &ViewMirror, i: usize) -> Pos2 {
        self.to_screen(view, self.input_pin_w(sc, i))
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
            if let Some(f) = &self.frame_drag {
                out.extend(f.inside.iter().copied());
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

    fn near(p: Pos2, c: Pos2, grab: f32) -> Option<f32> {
        let d2 = (p - c).length_sq();
        (d2 <= grab * grab).then_some(d2)
    }

    fn hit_node(&mut self, sc: &Scene, view: &ViewMirror, p: Pos2) -> Option<usize> {
        let cands = self.candidates(sc, view, p, 1.0);
        cands.into_iter().find(|&i| self.rect_of(sc, view, i).contains(p))
    }

    /// The nearest node within the grab radius of its box, `except` skipped.
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
            if let Some(d2) = Self::near(p, self.output_pin(sc, view, node, k), grab)
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

    fn hit_action(
        &self,
        sc: &Scene,
        view: &ViewMirror,
        node: usize,
        p: Pos2,
        detail: Detail,
    ) -> Option<ActHit> {
        let n = sc.nodes.get(node)?;
        if detail != Detail::Full || n.b.actions == 0 || n.collapsed {
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
    /// and the y of the line that shows it.
    fn drop_target(
        &mut self,
        sc: &Scene,
        view: &ViewMirror,
        g: &Graph,
        p: Pos2,
        detail: Detail,
    ) -> Option<(usize, bool, usize, f32)> {
        let Some(node) = self.hit_node(sc, view, p) else {
            // just under a node: the end of what it runs on exit, whether it has such actions or not
            let s = Self::scale(view);
            let reach = ACTION_ROW * s;
            let below = self.candidates(sc, view, p, reach).into_iter().find(|&i| {
                let r = self.rect_of(sc, view, i);
                p.x >= r.min.x && p.x <= r.max.x && p.y > r.max.y && p.y <= r.max.y + reach
            })?;
            let n = sc.nodes.get(below)?;
            if n.trigger || detail != Detail::Full || n.collapsed {
                return None;
            }
            let gn = g.find_node(&n.id)?;
            return Some((below, true, gn.on_exit.len(), self.rect_of(sc, view, below).max.y));
        };
        let n = sc.nodes.get(node)?;
        if n.trigger || detail != Detail::Full || n.collapsed {
            return None;
        }
        let gn = g.find_node(&n.id)?;
        let s = Self::scale(view);
        let r = self.rect_of(sc, view, node);
        if self.hit_header(sc, view, node, p) {
            return Some((node, false, gn.on_enter.len(), r.min.y + HEADER * s));
        }
        let hit = self.hit_action(sc, view, node, p, detail)?;
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

    /// The world polyline of a wire now: the scene's, or one built again around nodes being
    /// dragged and a reroute being moved.
    fn link_points(&self, sc: &Scene, li: usize) -> Vec<[f32; 2]> {
        let Some(l) = sc.links.get(li) else { return Vec::new() };
        let ends_moved = self.moving()
            && (self.pos_of(sc, l.from as usize)
                != sc.nodes.get(l.from as usize).map_or([0.0; 2], |n| n.pos)
                || self.pos_of(sc, l.to as usize) != sc.nodes.get(l.to as usize).map_or([0.0; 2], |n| n.pos));
        if !ends_moved && !self.knots_moving(sc, li) {
            return l.points.clone();
        }
        let w0 = self.output_pin_w(sc, l.from as usize, l.pin_index);
        let w3 = self.input_pin_w(sc, l.to as usize);
        let via: Vec<[f32; 2]> = (0..l.via.len()).map(|k| self.knot_at(sc, li, k)).collect();
        let boxes: Vec<[f32; 4]> = [l.from as usize, l.to as usize]
            .iter()
            .filter_map(|&i| {
                sc.nodes.get(i).map(|n| {
                    let p = self.pos_of(sc, i);
                    [p[0], p[1], p[0] + n.size[0], p[1] + n.size[1]]
                })
            })
            .collect();
        let wcx = wires::WireCx { flow: sc.flow, style: sc.wires.style, diagonal: sc.wires.diagonal };
        wires::build(&wcx, w0, w3, &via, wires::Lanes::default(), &boxes, &mut Vec::new()).points
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
        let s = Self::scale(view);
        let w = self.to_world(view, p);
        let r = (LINK_GRAB + sc.wires.thickness) / s;
        let cands = self.visible_links(sc, [w[0] - r, w[1] - r, w[0] + r, w[1] + r]);
        let mut best = r;
        let mut hit = None;
        for i in cands {
            let pts = self.link_points(sc, i);
            let d = wires::distance(&pts, w);
            if d < best {
                best = d;
                hit = Some(i);
            }
        }
        hit
    }

    fn hit_knot(&self, sc: &Scene, view: &ViewMirror, p: Pos2, grab: f32) -> Option<(usize, usize)> {
        let w = self.to_world(view, p);
        let r = grab / Self::scale(view);
        for (li, l) in sc.links.iter().enumerate() {
            if l.via.is_empty() {
                continue;
            }
            for (k, v) in l.via.iter().enumerate() {
                if (v[0] - w[0]).abs() <= r && (v[1] - w[1]).abs() <= r {
                    return Some((li, k));
                }
            }
        }
        None
    }

    fn hit_frame(&self, sc: &Scene, view: &ViewMirror, p: Pos2) -> Option<(usize, bool)> {
        let w = self.to_world(view, p);
        let s = Self::scale(view);
        let handle = (FRAME_HANDLE / s.max(0.05)).max(FRAME_HANDLE);
        for f in sc.frames.iter().rev() {
            let r = self.frame_rect(f);
            if w[0] >= r[2] - handle && w[0] <= r[2] && w[1] >= r[3] - handle && w[1] <= r[3] {
                return Some((f.index, true));
            }
            if w[0] >= r[0] && w[0] <= r[2] && w[1] >= r[1] && w[1] <= r[1] + FRAME_TITLE.max(18.0 / s) {
                return Some((f.index, false));
            }
        }
        None
    }

    /// A frame's rectangle now: moved or resized by a gesture under way.
    fn frame_rect(&self, f: &scene::SceneFrame) -> [f32; 4] {
        let mut r = f.rect;
        if let Some((_, index, at)) = self.frame_settle
            && index == f.index
            && self.frame_drag.is_none()
        {
            return at;
        }
        if let Some(d) = self.frame_drag.as_ref().filter(|d| d.index == f.index) {
            if d.resize {
                r[2] = (r[2] + d.delta[0]).max(r[0] + 64.0);
                r[3] = (r[3] + d.delta[1]).max(r[1] + 48.0);
            } else {
                r = [r[0] + d.delta[0], r[1] + d.delta[1], r[2] + d.delta[0], r[3] + d.delta[1]];
            }
        }
        r
    }
}

mod draw;
mod input;
mod menus;

pub use menus::{PickWant, pick_kinds};

#[cfg(test)]
mod tests;
