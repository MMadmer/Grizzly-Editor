//! The graph tab (`UIQuestGraphWindow`): one [`gz_app::Document`] per file. Its toolbar, the
//! Graph panel | the canvas | the Details panel with the drawer under them, the banner of an
//! external change, the Shortcuts card, the Graph menu and the keys.
//!
//! The tab holds VIEW state; the graph lives in `gz_nq`'s store. The view the canvas changes at
//! once (centre, zoom, selection, the open action) is mirrored here and reaches the document
//! when the gesture is over; a change the document made (MCP, an undo, a duplicate selecting its
//! copies) is taken back into the mirror once no command of ours is on its way.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{CornerRadius, Event, Key, Rect, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use gz_app::settings::{PrefDef, PrefType};
use gz_app::{
    Chord, DocCx, Document, EditAction, EditorSettings, OpenRequest, PrefGroup, PrefKind, PrefRow,
    ShellBuilder,
};
use gz_base::{Origin, Rev};
use gz_editor::Editor;
use gz_nq::canvas::{Diagonal, Flow, WireSettings, WireStyle};
use gz_nq::defaults::GraphDefaults;
use gz_nq::doc::{CanvasOp, DocCmd, DocOp, ViewOp, ZOOM};
use gz_nq::index::FindHit;
use gz_nq::{Catalog, DocHandle, NqService};
use gz_query::Derived;
use gz_ui::menu::{MenuItem, menu_item};
use gz_ui::{Clip, Font, Icon, Kind, Tone, Toolbar, palette};
use parking_lot::Mutex;
use smol_str::SmolStr;

use crate::canvas::{Canvas, CanvasCx, CanvasPrefs, Op, ViewMirror};
use crate::derived::{DocData, Refresher};
use crate::dialogs::{self, ProjectFind, TaskRefs, TaskRename};
use crate::inspector::{Inspector, PanelCx};
use crate::look;
use crate::scene::{Scene, SceneInput};

pub const GRAPH_W: &str = "graph.panel_w";
pub const DETAILS_W: &str = "graph.details_w";
pub const DRAWER_H: &str = "graph.drawer_h";
pub const GRAPH_DEF: f32 = 280.0;
pub const GRAPH_MIN: f32 = 220.0;
pub const GRAPH_MAX: f32 = 480.0;
pub const DETAILS_DEF: f32 = 360.0;
pub const DETAILS_MIN: f32 = 280.0;
pub const DETAILS_MAX: f32 = 640.0;
const DRAWER_DEF: f32 = 200.0;
const DRAWER_MIN: f32 = 120.0;
const CANVAS_MIN: f32 = 400.0;
const CANVAS_MIN_H: f32 = 260.0;
const RAIL: f32 = 36.0;
const GAP: f32 = 6.0;
/// Under this width of the tab the Details panel folds to a rail like the Graph panel.
const DETAILS_FOLD: f32 = 1400.0;

/// What a row of the Shortcuts card names: a group's heading, a gesture, or actions whose keys
/// are read off the bindings as they are now (with a gesture after them, `extra`).
enum KeyRow {
    Head,
    Gesture(&'static str),
    Keys(&'static [&'static str], &'static str),
}

/// The Shortcuts card's rows.
const KEYS: [(KeyRow, &str); 43] = [
    (KeyRow::Head, "View"),
    (KeyRow::Keys(&["graph.frame_all"], ""), "Frame the whole graph"),
    (KeyRow::Keys(&["graph.frame_selection"], ""), "Frame the selection"),
    (KeyRow::Keys(&["graph.back", "graph.forward"], ""), "Back / forward through the view history"),
    (KeyRow::Keys(&["graph.bookmark_1"], ""), "Go to bookmark 1 (Ctrl+1 to Ctrl+9)"),
    (KeyRow::Keys(&["graph.set_bookmark_1"], ""), "Set bookmark 1 on the node (Ctrl+Shift+1 to 9)"),
    (KeyRow::Gesture("Right or middle drag"), "Pan"),
    (KeyRow::Gesture("Wheel"), "Zoom at the cursor"),
    (KeyRow::Head, "Nodes"),
    (KeyRow::Keys(&["graph.add_node"], "Right-click"), "Add a node at the cursor"),
    (KeyRow::Gesture("Click / Ctrl+Click"), "Select / add to the selection"),
    (KeyRow::Gesture("Drag on empty space"), "Select with a rectangle"),
    (KeyRow::Keys(&["graph.select_all"], ""), "Select every node"),
    (KeyRow::Keys(&["graph.select_connected"], ""), "Select what the selection is wired to"),
    (KeyRow::Keys(&["graph.rename"], ""), "Rename the node"),
    (KeyRow::Keys(&["edit.delete"], ""), "Delete the selection"),
    (KeyRow::Keys(&["edit.copy", "edit.paste"], ""), "Copy / paste, also between graphs"),
    (KeyRow::Keys(&["edit.duplicate"], ""), "Duplicate the selection"),
    (KeyRow::Keys(&["graph.duplicate_with_wires"], ""), "Duplicate, keeping the wires into it"),
    (KeyRow::Keys(&["graph.disable"], ""), "Disable or enable: the game passes through it"),
    (KeyRow::Keys(&["graph.collapse"], ""), "Collapse or expand to the header"),
    (KeyRow::Keys(&["graph.add_frame"], ""), "A comment frame around the selection"),
    (KeyRow::Keys(&["edit.undo", "edit.redo"], ""), "Undo / redo"),
    (KeyRow::Keys(&["graph.find", "graph.find_next"], ""), "Find, find next"),
    (KeyRow::Head, "Arrange"),
    (KeyRow::Keys(&["graph.align_left", "graph.align_right"], ""), "Align left / right"),
    (KeyRow::Keys(&["graph.align_top", "graph.align_bottom"], ""), "Align top / bottom"),
    (KeyRow::Keys(&["graph.straighten"], ""), "Straighten the wires out of the selection"),
    (KeyRow::Gesture("Drag a node"), "Snaps to the grid and to its neighbours' edges"),
    (KeyRow::Head, "Wires"),
    (KeyRow::Gesture("Drag from a pin"), "Draw a wire; dropped on empty space, pick a node"),
    (KeyRow::Gesture("Drop a node on a wire"), "Put it on the wire"),
    (KeyRow::Gesture("Double-click a wire"), "Add a reroute; drag it, double-click it away"),
    (KeyRow::Gesture("Alt+drag a wire's end"), "Wire it elsewhere"),
    (KeyRow::Gesture("Ctrl+right drag"), "Cut every wire the line crosses"),
    (KeyRow::Keys(&["graph.connect_nearest"], ""), "Wire the selected node to the nearest free input"),
    (KeyRow::Keys(&["graph.remove_wire"], "Alt+Click"), "Remove the wire under the cursor"),
    (KeyRow::Keys(&["graph.bypass"], ""), "Keep the node, close the chain over it (A-B-C becomes A-C)"),
    (KeyRow::Head, "Actions"),
    (KeyRow::Gesture("Drag an action row"), "Move it within the node, or to another node"),
    (KeyRow::Gesture("Double-click a row"), "Edit the action in the Details panel"),
    (KeyRow::Head, "Other"),
    (KeyRow::Gesture("Right-click / Escape"), "The menu of what is under the cursor / cancel a gesture"),
];

/// The keys a row of the card prints: its actions' keys as bound now, then its gesture.
fn key_text(row: &KeyRow, keys_of: &dyn Fn(&str) -> String) -> String {
    match row {
        KeyRow::Head => String::new(),
        KeyRow::Gesture(g) => (*g).to_owned(),
        KeyRow::Keys(ids, extra) => {
            let mut parts: Vec<String> = ids.iter().map(|id| keys_of(id)).filter(|k| !k.is_empty()).collect();
            if !extra.is_empty() {
                parts.push((*extra).to_owned());
            }
            if parts.is_empty() { "Not bound".to_owned() } else { parts.join(" / ") }
        }
    }
}

/// The document id of a graph tab: `quest:<rel>` or `beh:<rel>`, `/` separated (`DocId`).
#[must_use]
pub fn doc_id(wire: &str, behaviour: bool) -> String {
    format!("{}:{}", if behaviour { "beh" } else { "quest" }, wire.replace('\\', "/"))
}

/// Plugs the two document kinds into the shell, the graph's actions and Preferences > Graph.
pub fn install(b: &mut ShellBuilder) {
    b.document("quest", |ed, req| GraphTab::open(ed, req).map(|t| Box::new(t) as Box<dyn Document>));
    b.document("beh", |ed, req| GraphTab::open(ed, req).map(|t| Box::new(t) as Box<dyn Document>));
    crate::actions::install(b);
    b.pref_group(PrefGroup {
        page: "Graph",
        icon: Icon::Quest,
        section: "Flow",
        rows: &PREF_FLOW,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Graph",
        icon: Icon::Quest,
        section: "Wires",
        rows: &PREF_WIRES,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Graph",
        icon: Icon::Quest,
        section: "Moving Nodes",
        rows: &PREF_MOVE,
        note: None,
    });
}

const fn pref(key: &'static str, ty: PrefType, default: f64, lo: f64, hi: f64) -> PrefDef {
    PrefDef { key, ty, default, lo, hi }
}

pub const P_FLOW: PrefDef = pref("graph_flow", PrefType::U32, 0.0, 0.0, 1.0);
pub const P_STYLE: PrefDef = pref("graph_wire_style", PrefType::U32, 0.0, 0.0, 4.0);
pub const P_DIAGONAL: PrefDef = pref("graph_wire_diagonal", PrefType::U32, 1.0, 0.0, 2.0);
pub const P_THICKNESS: PrefDef = pref("graph_wire_thickness", PrefType::Float, 2.0, 1.0, 4.0);
pub const P_BY_PIN: PrefDef = pref("graph_wire_by_pin", PrefType::Bool, 1.0, 0.0, 1.0);
pub const P_DIM: PrefDef = pref("graph_wire_dim", PrefType::Bool, 0.0, 0.0, 1.0);
pub const P_BUBBLES: PrefDef = pref("graph_wire_bubbles", PrefType::Bool, 0.0, 0.0, 1.0);
pub const P_ARROWS: PrefDef = pref("graph_wire_arrows", PrefType::Bool, 1.0, 0.0, 1.0);
pub const P_SNAP: PrefDef = pref("graph_snap", PrefType::Bool, 1.0, 0.0, 1.0);
pub const P_GUIDES: PrefDef = pref("graph_guides", PrefType::Bool, 1.0, 0.0, 1.0);

const fn row(
    label: &'static str,
    help: &'static str,
    def: PrefDef,
    unit: &'static str,
    choices: &'static [&'static str],
) -> PrefRow {
    PrefRow { label, help, kind: PrefKind::Setting { def, unit, choices } }
}

const PREF_FLOW: [PrefRow; 1] = [row(
    "Flow",
    "Which way a new graph runs: left to right (inputs on a node's left, outputs on its right) or top to bottom. A graph keeps its own; Graph > Flow Direction switches one.",
    P_FLOW,
    "",
    &["Left to Right", "Top to Bottom"],
)];

const PREF_WIRES: [PrefRow; 7] = [
    row(
        "Wire Style",
        "How wires are drawn: curved, straight, circuit (right angles on lanes of their own), angled (45-degree diagonals) or subway (angled, sharing a trunk). Graph > Wires sets a graph's own.",
        P_STYLE,
        "",
        &["Curved", "Straight", "Circuit", "Angled", "Subway"],
    ),
    row(
        "Diagonal Position",
        "Where an angled or subway wire takes its diagonal.",
        P_DIAGONAL,
        "",
        &["Start", "Middle", "End"],
    ),
    row("Thickness", "The wires' width at 100%; never under 1 px on screen.", P_THICKNESS, "px", &[]),
    row(
        "Colour by Pin",
        "A failure, timeout or abort wire in the danger hue, the other ways out by their place; off, every wire is neutral.",
        P_BY_PIN,
        "",
        &[],
    ),
    row("Dim Unrelated", "Wires that do not touch the selection fade to 15%.", P_DIM, "", &[]),
    row("Flow Bubbles", "Dots travel along the wires downstream of the selection.", P_BUBBLES, "", &[]),
    row("Arrowheads", "An arrowhead where a wire enters a node.", P_ARROWS, "", &[]),
];

const PREF_MOVE: [PrefRow; 2] = [
    row("Snap to Grid", "A dragged node lands on the 16 pt grid.", P_SNAP, "", &[]),
    row(
        "Alignment Guides",
        "A dragged node snaps to its neighbours' edges and centres, and a guide shows it.",
        P_GUIDES,
        "",
        &[],
    ),
];

/// What Preferences > Graph says now: the flow of a new graph, the wires, the dragging.
fn read_prefs(ed: &Editor) -> (Flow, WireSettings, CanvasPrefs) {
    let s = EditorSettings::of(ed);
    let get = |d: &PrefDef| s.pref(d.key).unwrap_or(d.default);
    let on = |d: &PrefDef| get(d) != 0.0;
    let flow = if get(&P_FLOW) as u32 == 1 { Flow::Down } else { Flow::Right };
    let style = WireStyle::ALL.get(get(&P_STYLE) as usize).copied().unwrap_or(WireStyle::Curved);
    let diagonal = Diagonal::ALL.get(get(&P_DIAGONAL) as usize).copied().unwrap_or(Diagonal::Middle);
    let wires = WireSettings {
        style,
        diagonal,
        thickness: (get(&P_THICKNESS) as f32).clamp(1.0, 4.0),
        by_pin: on(&P_BY_PIN),
        dim: on(&P_DIM),
        bubbles: on(&P_BUBBLES),
        arrows: on(&P_ARROWS),
    };
    (flow, wires, CanvasPrefs { snap: on(&P_SNAP), guides: on(&P_GUIDES) })
}

/// Preferences > Graph reach the documents (a new graph's flow, the wires of `graph.get`).
fn sync_defaults(ed: &Editor) -> (WireSettings, CanvasPrefs) {
    let (flow, wires, prefs) = read_prefs(ed);
    let d = GraphDefaults::of(ed);
    if d.flow() != flow || d.wires() != wires {
        d.set(flow, wires);
    }
    (wires, prefs)
}

/// The drawer under the panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerTab {
    None,
    Problems,
    Find,
}

impl DrawerTab {
    #[must_use]
    pub fn of(name: &str) -> Self {
        match name {
            "problems" => Self::Problems,
            "find" => Self::Find,
            _ => Self::None,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Problems => "problems",
            Self::Find => "find",
        }
    }
}

type Slot = Arc<Mutex<Option<Result<Arc<DocHandle>, String>>>>;

/// One graph tab.
pub struct GraphTab {
    editor: Arc<Editor>,
    id: String,
    wire: String,
    behaviour: bool,
    slot: Slot,
    doc: Option<Arc<DocHandle>>,
    load_error: Option<String>,
    refresher: Refresher,
    data: DocData,
    scenes: Derived<Scene>,
    scene: Option<Arc<Scene>>,
    finds: Derived<Vec<FindHit>>,
    pub canvas: Canvas,
    pub inspector: Inspector,
    pub mirror: ViewMirror,
    synced: ViewMirror,
    pending: Arc<AtomicUsize>,
    errors: Arc<Mutex<Vec<String>>>,
    queued: Vec<Op>,
    last_status_rev: Option<String>,
    // panes
    pub show_graph: bool,
    pub show_details: bool,
    pub show_keys: bool,
    graph_overlay: bool,
    rail: bool,
    /// The Details panel folded to a rail (a narrow tab), and opened over the canvas from it.
    pub details_rail: bool,
    pub details_overlay: bool,
    /// Preferences > Graph as the last frame read them.
    wires: WireSettings,
    prefs: CanvasPrefs,
    /// Where the picker keeps this project's Recent.
    pick_memory: String,
    pub pane_graph: Option<Rect>,
    pub pane_canvas: Option<Rect>,
    /// What the last frame drew over the canvas: the Keyboard Shortcuts card, the rail's Graph panel.
    over_canvas: Vec<Rect>,
    /// The Keyboard Shortcuts card as the last frame laid it out.
    keys_rect: Option<Rect>,
    pub pane_details: Option<Rect>,
    pub pane_drawer: Option<Rect>,
    /// Frames a quest_view change still owes before its answer describes it.
    pub view_frames: u32,
    pub drawn_frame: u64,
    message: String,
    message_tone: Tone,
    find_text: String,
    find_focus: bool,
    pending_rename: bool,
    project_find: ProjectFind,
    task_rename: TaskRename,
    task_refs: TaskRefs,
    drawn_sent: Option<([u32; 2], u32, bool)>,
    /// The tab asked for a file of another graph to come up with a node framed.
    open_after: Option<(String, String)>,
    clipboard: Option<arboard::Clipboard>,
    force_refresh: bool,
}

impl GraphTab {
    /// The factory: the document of a path, opened on the IO pool when it is not open yet.
    pub fn open(editor: &Arc<Editor>, req: &OpenRequest) -> Result<GraphTab, String> {
        let path = req
            .path
            .clone()
            .or_else(|| req.id.as_ref().and_then(|i| i.split_once(':').map(|(_, p)| p.to_owned())))
            .ok_or_else(|| "path= names the graph (project-relative)".to_owned())?;
        let svc = NqService::of(editor).map_err(|e| e.message)?;
        let p = svc.resolve(&path, false)?;
        // the preferences reach the document before its file is read: a new graph takes the flow
        let (wires, prefs) = sync_defaults(editor);
        let pick_memory = {
            use std::hash::{Hash, Hasher};
            let abs = p.abs.to_string_lossy().replace('\\', "/").to_lowercase();
            let root = abs.strip_suffix(&p.wire.to_lowercase()).unwrap_or(&abs).to_owned();
            let mut h = rustc_hash::FxHasher::default();
            root.hash(&mut h);
            format!("graph.{:016x}", h.finish())
        };
        let behaviour = gz_nq::asset::is_behaviour_path(&p.wire);
        let slot: Slot = Arc::new(Mutex::new(None));
        let doc = svc.get_doc(&p.wire);
        if doc.is_none() {
            let (ed, s, wire) = (editor.clone(), slot.clone(), p.wire.clone());
            let _task = editor.exec.spawn(async move {
                let r = match NqService::of(&ed) {
                    Ok(svc) => svc.open(&ed, &wire).await,
                    Err(e) => Err(e.message),
                };
                *s.lock() = Some(r);
                if let Ok(q) = gz_app::UiQueue::of(&ed) {
                    q.post(|_, ctx| ctx.request_repaint());
                }
            });
        }
        let frame_on_open = doc.as_ref().is_none_or(|d| {
            let v = &d.store.snapshot().view;
            v.center == [0.0, 0.0]
        });
        let mirror = doc.as_ref().map(|d| ViewMirror::of(&d.store.snapshot().view)).unwrap_or_default();
        Ok(GraphTab {
            editor: editor.clone(),
            id: doc_id(&p.wire, behaviour),
            wire: p.wire.clone(),
            behaviour,
            slot,
            doc,
            load_error: None,
            refresher: Refresher::default(),
            data: DocData::default(),
            scenes: Derived::new(editor.exec.clone()),
            scene: None,
            finds: Derived::new(editor.exec.clone()),
            canvas: Canvas::new(&p.wire, frame_on_open),
            inspector: Inspector::new(&p.wire),
            synced: mirror.clone(),
            mirror,
            pending: Arc::new(AtomicUsize::new(0)),
            errors: Arc::new(Mutex::new(Vec::new())),
            queued: Vec::new(),
            last_status_rev: None,
            show_graph: true,
            show_details: true,
            show_keys: false,
            graph_overlay: false,
            rail: false,
            details_rail: false,
            details_overlay: false,
            wires,
            prefs,
            pick_memory,
            pane_graph: None,
            pane_canvas: None,
            over_canvas: Vec::new(),
            keys_rect: None,
            pane_details: None,
            pane_drawer: None,
            view_frames: 0,
            drawn_frame: 0,
            message: String::new(),
            message_tone: Tone::Muted,
            find_text: String::new(),
            find_focus: false,
            pending_rename: false,
            project_find: ProjectFind::default(),
            task_rename: TaskRename::default(),
            task_refs: TaskRefs::default(),
            drawn_sent: None,
            open_after: None,
            clipboard: None,
            force_refresh: false,
        })
    }

    #[must_use]
    pub fn handle(&self) -> Option<&Arc<DocHandle>> {
        self.doc.as_ref()
    }

    #[must_use]
    pub fn wire(&self) -> &str {
        &self.wire
    }

    /// The tab's id: `quest:<path>` or `beh:<path>`.
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.id
    }

    /// A view command of the document (the history, a bookmark, the minimap).
    pub fn post_view(&self, ctx: &egui::Context, op: ViewOp) {
        self.post(ctx, DocOp::View(op));
    }

    #[must_use]
    pub fn behaviour(&self) -> bool {
        self.behaviour
    }

    #[must_use]
    pub fn scene(&self) -> Option<&Arc<Scene>> {
        self.scene.as_ref()
    }

    #[must_use]
    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        self.data.catalog.clone()
    }

    /// Commands of ours the document has not applied yet.
    #[must_use]
    pub fn ops_pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// The document whatever this tab has drawn so far: its own handle once a frame took it,
    /// else the service's. The service holds the one handle of a path, and a command over MCP
    /// reaches the document before any frame of the tab - so what closes, saves or discards a
    /// tab never asks the tab's own copy alone.
    fn shared(&self) -> Option<Arc<DocHandle>> {
        self.doc.clone().or_else(|| NqService::of(&self.editor).ok()?.get_doc(&self.wire))
    }

    fn take_loaded(&mut self) {
        if self.doc.is_some() {
            return;
        }
        let Some(r) = self.slot.lock().take() else { return };
        match r {
            Ok(d) => {
                self.mirror = ViewMirror::of(&d.store.snapshot().view);
                self.synced = self.mirror.clone();
                self.doc = Some(d);
            }
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Sends one command to the document, counted until it is applied.
    fn post(&self, ctx: &egui::Context, op: DocOp) {
        let (Some(doc), Some(catalog)) = (&self.doc, &self.data.catalog) else { return };
        self.pending.fetch_add(1, Ordering::AcqRel);
        let pending = self.pending.clone();
        let errors = self.errors.clone();
        let ctx = ctx.clone();
        doc.store.post_then(
            &self.editor.exec,
            DocCmd { catalog: catalog.clone(), op },
            self.editor.ui_origin(),
            move |r| {
                pending.fetch_sub(1, Ordering::AcqRel);
                if let Err(e) = r {
                    errors.lock().push(e.message);
                }
                ctx.request_repaint();
            },
        );
    }

    fn undo_redo(&self, ctx: &egui::Context, undo: bool) {
        let Some(doc) = &self.doc else { return };
        self.pending.fetch_add(1, Ordering::AcqRel);
        let (store, pending, ctx) = (doc.store.clone(), self.pending.clone(), ctx.clone());
        let _task = self.editor.exec.spawn(async move {
            if undo {
                store.undo().await;
            } else {
                store.redo().await;
            }
            pending.fetch_sub(1, Ordering::AcqRel);
            ctx.request_repaint();
        });
    }

    /// The selection and the open action reach the document before a command that reads them.
    fn flush_selection(&mut self, ctx: &egui::Context) {
        if self.mirror.selection != self.synced.selection {
            self.post(
                ctx,
                DocOp::View(ViewOp::Select(self.mirror.selection.iter().map(ToString::to_string).collect())),
            );
            self.synced.selection.clone_from(&self.mirror.selection);
            self.synced.slot.clear();
        }
        if self.mirror.slot != self.synced.slot {
            let s = if self.mirror.slot.is_empty() { "none".to_owned() } else { self.mirror.slot.clone() };
            self.post(ctx, DocOp::View(ViewOp::Slot(s)));
            self.synced.slot.clone_from(&self.mirror.slot);
        }
    }

    /// The centre and the zoom reach the document once nothing moves them any more.
    fn flush_view(&mut self, ctx: &egui::Context) {
        if self.canvas.view_moving {
            return;
        }
        if self.mirror.zoom != self.synced.zoom {
            self.post(ctx, DocOp::View(ViewOp::Zoom(self.mirror.zoom)));
            self.synced.zoom = self.mirror.zoom;
        }
        #[allow(clippy::float_cmp)]
        if self.mirror.center != self.synced.center {
            self.post(ctx, DocOp::View(ViewOp::Center(self.mirror.center)));
            self.synced.center = self.mirror.center;
        }
    }

    /// What the canvas and the panels asked for this frame.
    fn run_ops(&mut self, ctx: &egui::Context, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        self.flush_selection(ctx);
        for op in ops {
            match op {
                Op::Doc(d) => self.post(ctx, d),
                Op::FrameNode(id) => self.frame_node(ctx, &id),
                Op::Undo => self.undo_redo(ctx, true),
                Op::Redo => self.undo_redo(ctx, false),
                Op::Slot(s) => self.mirror.slot = s,
            }
        }
    }

    /// Centres a node at 1:1 or closer and selects it: one step of the view history.
    pub fn frame_node(&mut self, ctx: &egui::Context, id: &str) {
        let Some(n) = self
            .scene
            .as_ref()
            .and_then(|s| s.node_of(id).and_then(|i| s.nodes.get(i)))
            .map(|n| (n.pos, n.size))
        else {
            self.post(ctx, DocOp::View(ViewOp::FrameNode(id.to_owned())));
            return;
        };
        self.canvas.cancel_framing();
        self.mirror.center = [n.0[0] + n.1[0] * 0.5, n.0[1] + n.1[1] * 0.5];
        self.mirror.zoom = self.mirror.zoom.max(gz_nq::doc::ZOOM_DEFAULT);
        self.mirror.select_only(id);
        self.post(ctx, DocOp::View(ViewOp::FrameNode(id.to_owned())));
        self.synced = self.mirror.clone();
    }

    fn noun(&self) -> &'static str {
        look::doc_noun(self.behaviour)
    }

    fn set_message(&mut self, text_: &str, tone: Tone) {
        text_.clone_into(&mut self.message);
        self.message_tone = tone;
    }

    // ---- the panes ----------------------------------------------------------------------------

    /// Asks for the Graph panel (`SetGraphPanel`): on a rail it is the one over the canvas.
    pub fn set_graph_panel(&mut self, on: bool) {
        if self.rail && self.show_graph {
            self.graph_overlay = on;
            return;
        }
        self.show_graph = on;
        self.graph_overlay = on && self.rail;
    }

    /// `open`, `rail`, `overlay` or `hidden`, as the last frame drew it.
    #[must_use]
    pub fn graph_panel_state(&self) -> &'static str {
        if !self.show_graph {
            "hidden"
        } else if !self.rail {
            "open"
        } else if self.graph_overlay {
            "overlay"
        } else {
            "rail"
        }
    }

    #[must_use]
    pub fn graph_on_screen(&self) -> bool {
        self.show_graph && (!self.rail || self.graph_overlay)
    }

    #[must_use]
    pub fn graph_width(&self, ctx: &egui::Context) -> f32 {
        gz_ui::layout::get(ctx, GRAPH_W, GRAPH_DEF).clamp(GRAPH_MIN, GRAPH_MAX)
    }

    #[must_use]
    pub fn details_width(&self, ctx: &egui::Context) -> f32 {
        self.pane_details.map_or_else(
            || gz_ui::layout::get(ctx, DETAILS_W, DETAILS_DEF).clamp(DETAILS_MIN, DETAILS_MAX),
            |r| r.width(),
        )
    }

    #[must_use]
    pub fn drawer(&self) -> DrawerTab {
        self.doc.as_ref().map_or(DrawerTab::None, |d| DrawerTab::of(&d.store.snapshot().view.drawer))
    }

    pub fn set_drawer(&mut self, ctx: &egui::Context, d: DrawerTab) {
        self.post(ctx, DocOp::View(ViewOp::Drawer(SmolStr::new(d.name()))));
    }

    /// Whether quest_view's `scroll=` has a pane on screen (`CanScroll`).
    pub fn can_scroll(&self, w: &str) -> Result<(), String> {
        let graph = self.graph_on_screen();
        if w == "tasks" || w == "variables" || w.starts_with("task:") {
            if !graph {
                return Err("the Graph panel is not on screen: quest_view graph_panel=show first".into());
            }
            if let Some(rest) = w.strip_prefix("task:") {
                if self.behaviour {
                    return Err("a behaviour graph has roles, not tasks".into());
                }
                let id = rest.split('/').next().unwrap_or(rest);
                let exists =
                    self.doc.as_ref().is_some_and(|d| d.store.snapshot().graph.find_task(id).is_some());
                if !exists {
                    return Err(format!("no task '{id}' in this quest"));
                }
            }
            return Ok(());
        }
        if w == "top" || w == "bottom" {
            if self.mirror.selection.len() == 1 {
                if !self.show_details {
                    return Err("the Details panel is hidden: quest_view details=show first".into());
                }
                return Ok(());
            }
            if !graph {
                return Err("nothing is selected and the Graph panel is not on screen".into());
            }
            return Ok(());
        }
        if w.starts_with("action:") {
            if !self.show_details {
                return Err("the Details panel is hidden: quest_view details=show first".into());
            }
            if !Inspector::has_action(&self.mirror) {
                return Err("no action is open: pass slot=enter:N or slot=exit:N".into());
            }
            return Ok(());
        }
        Err("scroll must be top, bottom, action:top, action:bottom, tasks, task:<id>, task:<id>/steps or variables".into())
    }

    /// The tab was drawn in the frame being built or the one before.
    #[must_use]
    pub fn drawn_lately(&self, ctx: &egui::Context) -> bool {
        self.drawn_frame + 1 >= ctx.cumulative_frame_nr()
    }

    pub fn owe_frames(&mut self, n: u32) {
        self.view_frames = self.view_frames.max(n);
    }

    #[must_use]
    pub fn view_pending(&self) -> bool {
        // a tab that came up a moment ago and was not drawn yet has no panes to answer
        self.view_frames > 0
            || self.canvas.frame_pending()
            || self.ops_pending() > 0
            || self.pane_canvas.is_none()
    }

    /// Opens the find drawer with the field focused.
    pub fn open_find(&mut self, ctx: &egui::Context) {
        self.set_drawer(ctx, DrawerTab::Find);
        self.find_focus = true;
    }

    fn find_results(&self) -> (Vec<FindHit>, i64) {
        let Some(doc) = &self.doc else { return (Vec::new(), -1) };
        let snap = doc.store.snapshot();
        let Some(cat) = self.data.catalog.clone() else { return (Vec::new(), -1) };
        let q = snap.view.find_query.clone();
        let rev = Rev(snap.edit).mix(Rev(gz_nq::index::content_hash(&q))).mix(Rev(u64::from(cat.generation)));
        let graph = snap.graph.clone();
        let hits = self.finds.get(rev, move || gz_nq::doc::find_results(&cat, &graph, &q));
        let results = hits.value().map(|v| (**v).clone()).unwrap_or_default();
        let index = snap.find_index(&results);
        (results, index)
    }

    /// Goes to a find result (wrapping); -1 is the last (`NavigateFind`).
    pub fn navigate_find(&mut self, ctx: &egui::Context, to: i64) {
        self.post(ctx, DocOp::View(ViewOp::FindGo(to)));
    }

    // ---- what the frame draws -----------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn draw_body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        self.take_loaded();
        let Some(doc) = self.doc.clone() else {
            let mut c = ui.new_child(UiBuilder::new().max_rect(full.shrink(16.0)));
            match &self.load_error {
                Some(e) => {
                    gz_ui::state::empty_state(
                        &mut c,
                        Icon::Error,
                        &format!("The {} could not be opened", self.noun()),
                        e,
                        None,
                    );
                }
                None => gz_ui::state::busy_line(&mut c, "Reading the file"),
            }
            ui.allocate_rect(full, egui::Sense::hover());
            return;
        };
        self.drawn_frame = ctx.cumulative_frame_nr();
        let snap = doc.store.snapshot();
        let (wires, prefs) = sync_defaults(&self.editor);
        self.wires = wires;
        self.prefs = prefs;
        let force = std::mem::take(&mut self.force_refresh);
        self.refresher.tick(&self.editor, &doc, &ctx, snap.edit, force);
        self.data = self.refresher.get();
        for e in std::mem::take(&mut *self.errors.lock()) {
            self.canvas.set_status(&ctx, &e);
            self.editor.log.info(format!("[nq] {e}"));
        }
        for n in std::mem::take(&mut self.inspector.notes) {
            self.canvas.set_status(&ctx, &n);
        }
        // the document moved the view (MCP, undo, a command that selects): the mirror follows
        let store_view = ViewMirror::of(&snap.view);
        if self.ops_pending() == 0 && store_view != self.synced && !self.canvas.view_moving {
            self.mirror = store_view.clone();
            self.synced = store_view;
        }
        let Some(cat) = self.data.catalog.clone() else {
            let mut c = ui.new_child(UiBuilder::new().max_rect(full.shrink(16.0)));
            gz_ui::state::busy_line(&mut c, "Reading the catalog");
            ui.allocate_rect(full, egui::Sense::hover());
            return;
        };
        // the scene of this revision, built on the CPU pool
        let p = palette(&ctx);
        // the wires in force: the graph's own over the preferences
        let set = snap.graph.canvas.wires.over(&self.wires);
        let rev = Rev(snap.edit)
            .mix(Rev(u64::from(cat.generation)))
            .mix(Rev(self.data.names_rev))
            .mix(Rev(wire_key(&set)));
        let (s2, c2, p2, n2) = (snap.clone(), cat.clone(), p.clone(), self.data.names.clone());
        let ans = self.scenes.get(rev, move || {
            Scene::build(&SceneInput {
                graph: &s2.graph,
                catalog: &c2,
                edit: s2.edit,
                palette: &p2,
                wires: set,
                names: Some(&*n2),
            })
        });
        self.scene = ans.value().cloned();
        if !ans.is_fresh() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        let queued = std::mem::take(&mut self.queued);
        self.run_ops(&ctx, queued);
        let doc_status = (snap.view.status != self.last_status_rev.clone().unwrap_or_default())
            .then(|| snap.view.status.clone())
            .filter(|s| !s.is_empty());
        self.last_status_rev = Some(snap.view.status.clone());

        // the panes: the panels keep their pt size, the canvas takes what is left
        let origin = full.min;
        let mut top = origin.y;
        let bottom = full.max.y;
        // what the file on disk says that the graph does not show: it does not read, it is gone,
        // or both it and the unsaved edits changed some values
        let disk = &self.data.disk;
        if disk.error.is_some() || disk.missing || !disk.conflicts.is_empty() {
            let banner_r =
                Rect::from_min_max(pos2(origin.x + 8.0, top + 8.0), pos2(full.max.x - 8.0, bottom));
            let mut b = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.banner").max_rect(banner_r));
            let name = gz_nq::asset::base_name(&self.wire);
            let file = std::path::Path::new(&self.wire)
                .file_name()
                .map_or(name, |f| f.to_string_lossy().into_owned());
            if let Some(e) = &disk.error {
                let text_ = format!(
                    "{file} does not read: {e}. The graph shows the last state that read, until the file is fixed."
                );
                gz_ui::state::banner(&mut b, Tone::Error, &text_, None, None);
            } else if disk.missing {
                let text_ = format!("{file} is gone from the disk. Save writes it again.");
                gz_ui::state::banner(&mut b, Tone::Warning, &text_, None, None);
            } else {
                let n = disk.conflicts.len();
                let text_ = format!(
                    "Changed on disk too: {n} value{}. The graph keeps its own; Take Theirs takes the file's.",
                    if n == 1 { "" } else { "s" }
                );
                let lines: Vec<String> = disk.conflicts.iter().map(conflict_line).collect();
                let items: Vec<(&str, Option<&str>)> =
                    lines.iter().map(|l| (l.as_str(), Some("Take Theirs"))).collect();
                if let Some(i) = gz_ui::state::banner_list(&mut b, Tone::Warning, &text_, &items) {
                    self.take_theirs(&ctx, i);
                }
            }
            top = b.min_rect().max.y + 8.0 + GAP;
        }
        let drawer = self.drawer();
        let drawer_h = if drawer == DrawerTab::None {
            0.0
        } else {
            let room = bottom - top - CANVAS_MIN_H - GAP;
            gz_ui::layout::get(&ctx, DRAWER_H, DRAWER_DEF).min(room.max(DRAWER_MIN)).max(DRAWER_MIN)
        };
        let main_h = (bottom - top - if drawer_h > 0.0 { drawer_h + GAP } else { 0.0 }).max(60.0);
        let width = full.width();
        self.details_rail = self.show_details && width < DETAILS_FOLD;
        if !self.details_rail {
            self.details_overlay = false;
        }
        let mut dw = if self.details_rail {
            RAIL
        } else if self.show_details {
            gz_ui::layout::get(&ctx, DETAILS_W, DETAILS_DEF).clamp(DETAILS_MIN, DETAILS_MAX)
        } else {
            0.0
        };
        let mut gw = if self.show_graph { self.graph_width(&ctx) } else { 0.0 };
        let gaps = f32::from(u8::from(self.show_graph) + u8::from(self.show_details));
        self.rail = self.show_graph && width - gw - dw - GAP * gaps < CANVAS_MIN;
        if self.rail {
            gw = RAIL;
        } else {
            self.graph_overlay = false;
        }
        if self.show_details && !self.details_rail && width - gw - dw - GAP * gaps < CANVAS_MIN {
            dw = (width - gw - GAP * gaps - CANVAS_MIN).max(DETAILS_MIN);
        }
        let canvas_w = (width - gw - dw - GAP * gaps).max(80.0);
        let spare = width - gw - dw - GAP * gaps - CANVAS_MIN;
        let graph_hi = (gw + spare).clamp(GRAPH_MIN, GRAPH_MAX);
        let details_hi = (dw + spare).clamp(DETAILS_MIN, DETAILS_MAX);

        let mut ops: Vec<Op> = Vec::new();
        let mut canvas_actions: Vec<&'static str> = Vec::new();
        let pending = self.ops_pending() > 0;
        let mut x = origin.x;
        self.pane_graph = None;
        if self.show_graph {
            let r = Rect::from_min_size(pos2(x, top), vec2(gw, main_h));
            if self.rail {
                self.draw_rail(ui, r);
                let lx = x + gw + GAP * 0.5;
                ui.painter()
                    .line_segment([pos2(lx, top), pos2(lx, top + main_h)], Stroke::new(1.0, p.border_subtle));
            } else {
                ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
                let mut pcx = PanelCx {
                    editor: &self.editor,
                    cat: &cat,
                    graph: &snap.graph,
                    edit: snap.edit,
                    data: &self.data,
                    view: &mut self.mirror,
                    ops: &mut ops,
                    pending,
                };
                self.inspector.draw_graph_panel(ui, r.shrink(8.0), &mut pcx);
                self.pane_graph = Some(r);
                gz_ui::splitter::splitter_at(
                    ui,
                    GRAPH_W,
                    true,
                    pos2(x + gw + GAP * 0.5, top),
                    main_h,
                    1.0,
                    GRAPH_MIN,
                    graph_hi,
                    GRAPH_DEF,
                );
            }
            x += gw + GAP;
        }

        // the canvas, edge to edge
        let canvas_r = Rect::from_min_size(pos2(x, top), vec2(canvas_w, main_h));
        self.pane_canvas = Some(canvas_r);
        {
            let mut cui = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.canvas").max_rect(canvas_r));
            cui.set_clip_rect(canvas_r.intersect(ui.clip_rect()));
            let covered = std::mem::take(&mut self.over_canvas);
            let mut ccx = CanvasCx {
                scene: self.scene.as_deref(),
                graph: &snap.graph,
                catalog: &cat,
                data: &self.data,
                view: &mut self.mirror,
                minimap: snap.view.minimap,
                ops: &mut ops,
                actions: &mut canvas_actions,
                editor: Some(&self.editor),
                prefs: self.prefs,
                edit: snap.edit,
                doc_status,
                covered: &covered,
                pick_memory: &self.pick_memory,
            };
            self.canvas.draw(&mut cui, canvas_r, &mut ccx);
            // Ctrl+C / Ctrl+X / Ctrl+V reach the canvas as clipboard events, not keys
            if self.canvas.takes_keys(&ctx) {
                let events: Vec<Event> = ctx.input(|i| {
                    i.events
                        .iter()
                        .filter(|e| matches!(e, Event::Copy | Event::Cut | Event::Paste(_)))
                        .cloned()
                        .collect()
                });
                for e in events {
                    match e {
                        Event::Copy | Event::Cut => {
                            if let Some(t) = Canvas::copy_text(&snap.graph, &ccx.view.selection) {
                                ctx.copy_text(t);
                            }
                            if matches!(e, Event::Cut) && !ccx.view.selection.is_empty() {
                                ccx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Delete)));
                            }
                        }
                        Event::Paste(t) => {
                            self.canvas.paste(&mut ccx, &t, None);
                        }
                        _ => {}
                    }
                }
            }
        }
        x += canvas_w;
        // the empty canvas's Paste: the clipboard is read only now
        if let Some(at) = self.canvas.want_paste.take() {
            if self.clipboard.is_none() {
                self.clipboard = arboard::Clipboard::new().ok();
            }
            if let Some(text_) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) {
                self.paste_text(&text_, Some(at));
            }
        }
        // what the canvas's menus picked of the group Graph runs as the action it is
        for id in canvas_actions {
            cx.requests.action(id);
        }

        // F2 with the Details panel hidden brings it back to rename in
        let rename = std::mem::take(&mut self.canvas.want_rename) || self.pending_rename;
        let focus_action = std::mem::take(&mut self.canvas.want_focus_action);
        self.pending_rename = false;
        if (rename || focus_action) && (!self.show_details || (self.details_rail && !self.details_overlay)) {
            self.show_details = true;
            self.details_overlay = self.details_rail;
            self.pending_rename = rename;
        } else if rename {
            self.inspector.focus_rename = true;
        }
        if focus_action {
            self.inspector.focus_action = true;
        }
        self.pane_details = None;
        if self.details_rail {
            let r = Rect::from_min_size(pos2(x + GAP, top), vec2(RAIL, main_h));
            ui.painter().line_segment(
                [pos2(x + GAP * 0.5, top), pos2(x + GAP * 0.5, top + main_h)],
                Stroke::new(1.0, p.border_subtle),
            );
            self.draw_details_rail(ui, r);
            if self.details_overlay {
                let w = DETAILS_DEF.min(canvas_w);
                let or = Rect::from_min_size(pos2(canvas_r.max.x - w, top), vec2(w, main_h));
                let mut oui =
                    ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.details_overlay").max_rect(or));
                oui.painter().rect_filled(or, CornerRadius::ZERO, p.overlay);
                oui.painter().rect_stroke(
                    or,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, p.border),
                    StrokeKind::Inside,
                );
                oui.interact(
                    or,
                    egui::Id::unique(("gz-ui-graph.details_block", self.id.as_str())),
                    egui::Sense::click_and_drag(),
                );
                let mut pcx = PanelCx {
                    editor: &self.editor,
                    cat: &cat,
                    graph: &snap.graph,
                    edit: snap.edit,
                    data: &self.data,
                    view: &mut self.mirror,
                    ops: &mut ops,
                    pending,
                };
                self.inspector.draw_details(&mut oui, or.shrink(8.0), &mut pcx);
                self.pane_details = Some(or);
                self.over_canvas.push(or);
            }
        } else if self.show_details {
            gz_ui::splitter::splitter_at(
                ui,
                DETAILS_W,
                true,
                pos2(x + GAP * 0.5, top),
                main_h,
                -1.0,
                DETAILS_MIN,
                details_hi,
                DETAILS_DEF,
            );
            x += GAP;
            let r = Rect::from_min_size(pos2(x, top), vec2(dw, main_h));
            ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
            let mut pcx = PanelCx {
                editor: &self.editor,
                cat: &cat,
                graph: &snap.graph,
                edit: snap.edit,
                data: &self.data,
                view: &mut self.mirror,
                ops: &mut ops,
                pending,
            };
            self.inspector.draw_details(ui, r.shrink(8.0), &mut pcx);
            self.pane_details = Some(r);
        }

        // the rail's panel, over the canvas
        if self.rail && self.graph_overlay {
            let r = Rect::from_min_size(canvas_r.min, vec2(GRAPH_DEF.min(canvas_w), main_h));
            let mut oui = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.overlay").max_rect(r));
            oui.painter().rect_filled(r, CornerRadius::ZERO, p.overlay);
            oui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, p.border), StrokeKind::Inside);
            // the canvas under it takes no clicks meant for the panel
            oui.interact(
                r,
                egui::Id::unique(("gz-ui-graph.overlay_block", self.id.as_str())),
                egui::Sense::click_and_drag(),
            );
            let mut pcx = PanelCx {
                editor: &self.editor,
                cat: &cat,
                graph: &snap.graph,
                edit: snap.edit,
                data: &self.data,
                view: &mut self.mirror,
                ops: &mut ops,
                pending,
            };
            self.inspector.draw_graph_panel(&mut oui, r.shrink(8.0), &mut pcx);
            self.pane_graph = Some(r);
            self.over_canvas.push(r);
        }
        self.draw_keys_card(ui);
        self.over_canvas.extend(self.keys_rect);

        self.pane_drawer = None;
        if drawer != DrawerTab::None {
            let y = top + main_h;
            gz_ui::splitter::splitter_at(
                ui,
                DRAWER_H,
                false,
                pos2(origin.x, y + GAP * 0.5),
                width,
                -1.0,
                DRAWER_MIN,
                1000.0,
                DRAWER_DEF,
            );
            let r = Rect::from_min_size(pos2(origin.x, y + GAP), vec2(width, drawer_h));
            self.draw_drawer(ui, r, drawer, &snap.graph, &mut ops);
            self.pane_drawer = Some(r);
        }

        // the tab's dialogs belong to it, whichever panes are shown
        if let Some(id) = self.inspector.rename_task.take() {
            let mut pcx = PanelCx {
                editor: &self.editor,
                cat: &cat,
                graph: &snap.graph,
                edit: snap.edit,
                data: &self.data,
                view: &mut self.mirror,
                ops: &mut ops,
                pending,
            };
            self.inspector.flush(&mut pcx);
            self.task_rename.begin(&id);
        }
        if let Some(id) = self.inspector.refs_task.take() {
            let path = self.wire.clone();
            self.task_refs.begin(&self.editor, &ctx, &id, &path);
        }
        if let Some((node, exit, at)) = self.inspector.add_action.take() {
            self.canvas.request_add_action(&ctx, &cat, &snap.graph, &self.pick_memory, &node, exit, at);
        }
        let wire = self.wire.clone();
        self.task_rename.draw(ui, &self.editor, &wire);
        if let Some((node, slot)) = self.task_refs.draw(ui) {
            self.mirror.select_only(&node);
            self.mirror.slot = slot;
        }
        dialogs::draw_text_edit(ui, &mut self.inspector.text_edit);
        if let Some((path, node)) = self.project_find.draw(ui, &self.editor) {
            if gz_nq::asset::base_name(&path) == gz_nq::asset::base_name(&self.wire)
                && path.replace('\\', "/") == self.wire
            {
                self.frame_node(&ctx, &node);
            } else {
                self.open_after = Some((path.clone(), node));
                let mut req = OpenRequest::file(
                    if gz_nq::asset::is_behaviour_path(&path) { "beh" } else { "quest" },
                    &path,
                );
                req.id = Some(doc_id(&path, gz_nq::asset::is_behaviour_path(&path)));
                cx.requests.open(req);
                // the other tab frames the node when it comes up
                let (ed, id) = (self.editor.clone(), self.open_after.clone());
                if let Some((p2, n2)) = id {
                    let _task = self.editor.exec.spawn(async move {
                        if let Ok(svc) = NqService::of(&ed)
                            && let Ok(d) = svc.open(&ed, &p2).await
                        {
                            let _ = svc.send(&ed, &d, DocOp::View(ViewOp::FrameNode(n2)), Origin::Mcp).await;
                        }
                    });
                }
            }
        }
        {
            let mut pcx = PanelCx {
                editor: &self.editor,
                cat: &cat,
                graph: &snap.graph,
                edit: snap.edit,
                data: &self.data,
                view: &mut self.mirror,
                ops: &mut ops,
                pending,
            };
            self.inspector.commit_idle(&ctx, &mut pcx);
        }
        self.run_ops(&ctx, ops);
        self.flush_selection(&ctx);
        self.flush_view(&ctx);
        // what the frame drew, for quest_minimap
        let ppp = ctx.pixels_per_point();
        let drawn = (
            [(canvas_w * ppp) as u32, (main_h * ppp) as u32],
            (ppp * 100.0) as u32,
            self.canvas.minimap_shown,
        );
        if self.drawn_sent != Some(drawn) {
            self.drawn_sent = Some(drawn);
            self.post(
                &ctx,
                DocOp::View(ViewOp::Drawn {
                    canvas: Some([canvas_w * ppp, main_h * ppp]),
                    dpi: ppp,
                    minimap_shown: self.canvas.minimap_shown,
                }),
            );
        }
        if self.view_frames > 0 {
            self.view_frames -= 1;
            ctx.request_repaint();
        }
        ui.allocate_rect(full, egui::Sense::hover());
    }

    /// The Graph panel folded to a strip: the canvas keeps its 400 pt, the panel opens over it.
    fn draw_rail(&mut self, ui: &mut Ui, r: Rect) {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
        let mut rui = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-graph.rail")
                .max_rect(r.shrink(4.0))
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        rui.spacing_mut().item_spacing.y = 4.0;
        if gz_ui::icon_button(
            &mut rui,
            Icon::SidebarLeft,
            "The Graph panel, over the graph",
            self.graph_overlay,
            true,
        )
        .clicked()
        {
            self.graph_overlay = !self.graph_overlay;
        }
        let behaviour = self.behaviour;
        if gz_ui::icon_button(
            &mut rui,
            if behaviour { Icon::Role } else { Icon::Task },
            if behaviour { "Roles" } else { "Tasks" },
            false,
            true,
        )
        .clicked()
        {
            self.graph_overlay = true;
            self.inspector.scroll = Some("tasks".to_owned());
        }
        if gz_ui::icon_button(&mut rui, Icon::Variable, "Variables", false, true).clicked() {
            self.graph_overlay = true;
            self.inspector.scroll = Some("variables".to_owned());
        }
    }

    /// The Details panel folded to a strip: it opens over the canvas's right edge.
    fn draw_details_rail(&mut self, ui: &mut Ui, r: Rect) {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
        let mut rui = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-graph.details_rail")
                .max_rect(r.shrink(4.0))
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        rui.spacing_mut().item_spacing.y = 4.0;
        if gz_ui::icon_button(
            &mut rui,
            Icon::SidebarRight,
            "The Details panel, over the graph",
            self.details_overlay,
            true,
        )
        .clicked()
        {
            self.details_overlay = !self.details_overlay;
        }
    }

    /// `open`, `rail`, `overlay` or `hidden`, as the last frame drew it.
    #[must_use]
    pub fn details_state(&self) -> &'static str {
        if !self.show_details {
            "hidden"
        } else if !self.details_rail {
            "open"
        } else if self.details_overlay {
            "overlay"
        } else {
            "rail"
        }
    }

    /// What the keyboard does here, read off the canvas's keys: a card over the canvas.
    fn draw_keys_card(&mut self, ui: &mut Ui) {
        let Some(c) = self.pane_canvas.filter(|_| self.show_keys) else {
            self.keys_rect = None;
            return;
        };
        let editor = self.editor.clone();
        let keys_of = move |id: &str| gz_app::actions::keys_label(&editor, id);
        let card = keys_card(ui, c, self.keys_rect, self.id.as_str(), &keys_of);
        self.keys_rect = card.rect;
        if card.close {
            self.show_keys = false;
        }
    }

    #[allow(clippy::too_many_lines)]
    fn draw_drawer(
        &mut self,
        ui: &mut Ui,
        r: Rect,
        drawer: DrawerTab,
        graph: &gz_nq::Graph,
        ops: &mut Vec<Op>,
    ) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
        let inner =
            Rect::from_min_max(pos2(r.min.x + 8.0, r.min.y + 6.0), pos2(r.max.x - 8.0, r.max.y - 6.0));
        let mut dui = ui.new_child(UiBuilder::new().id_salt("gz-ui-graph.drawer").max_rect(inner));
        dui.set_clip_rect(inner.intersect(ui.clip_rect()));
        let (results, index) = self.find_results();
        let problems = format!("Problems ({})", self.data.problems.len());
        let found = format!("Find Results ({})", results.len());
        let mut cur = usize::from(drawer == DrawerTab::Find);
        let mut close = false;
        dui.horizontal(|ui| {
            if gz_ui::segmented(ui, &mut cur, &[&problems, &found], None) {
                // the switch is applied below
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = gz_ui::icon_button(ui, Icon::Close, "Close", false, true).clicked();
            });
        });
        let want = if cur == 1 { DrawerTab::Find } else { DrawerTab::Problems };
        if close {
            self.set_drawer(&ctx, DrawerTab::None);
            return;
        }
        if want != drawer {
            self.set_drawer(&ctx, want);
        }
        if want == DrawerTab::Find {
            let snap_query =
                self.doc.as_ref().map(|d| d.store.snapshot().view.find_query.clone()).unwrap_or_default();
            if (!dui.ctx().egui_wants_keyboard_input() || self.find_text.is_empty() && !snap_query.is_empty())
                && self.find_text != snap_query
                && !self.find_focus
            {
                self.find_text = snap_query.clone();
            }
            let mut go: Option<i64> = None;
            dui.horizontal(|ui| {
                let w = (ui.available_width() - 24.0 * 2.0 - 4.0 - gz_ui::units::em(5.0))
                    .max(gz_ui::units::em(8.0));
                let resp = gz_ui::SearchField::new(&mut self.find_text)
                    .hint("Find by id, kind, parameters, actions or comments")
                    .min_em(12.0)
                    .width(w)
                    .id_salt("gz-ui-graph.find")
                    .show(ui);
                if self.find_focus {
                    resp.request_focus();
                    self.find_focus = false;
                }
                if resp.changed() {
                    ops.push(Op::Doc(DocOp::View(ViewOp::FindQuery(self.find_text.clone()))));
                }
                // Enter in the field goes to the next result, Shift+Enter to the previous one
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = Some(if ui.input(|i| i.modifiers.shift) { index - 1 } else { index + 1 });
                    resp.request_focus();
                }
                if gz_ui::icon_button(ui, Icon::ChevronUp, "Previous (Shift+F3)", false, !results.is_empty())
                    .clicked()
                {
                    go = Some(index - 1);
                }
                if gz_ui::icon_button(ui, Icon::ChevronDown, "Next (F3)", false, !results.is_empty())
                    .clicked()
                {
                    go = Some(index + 1);
                }
                let count = if results.is_empty() {
                    "0".to_owned()
                } else {
                    format!("{} / {}", if index >= 0 { index + 1 } else { 0 }, results.len())
                };
                gz_ui::text::label(ui, &count, Font::Body, p.text_muted, Clip::End);
            });
            if results.is_empty() {
                gz_ui::text::status(
                    &mut dui,
                    Tone::Muted,
                    if self.find_text.is_empty() {
                        "Type to find nodes. Spaces separate terms, -term leaves a term out."
                    } else {
                        "No node matches."
                    },
                );
            } else {
                gz_ui::list::list(&mut dui, "gz-ui-graph.find_results", vec2(0.0, -2.0), |l| {
                    for (i, hit) in results.iter().enumerate() {
                        let text_ = format!("{}  \u{b7}  {}  \u{b7}  {}", hit.node, hit.title, hit.matched);
                        let selected = i64::try_from(i).ok() == Some(index);
                        if l.row(&format!("f{i}"), selected, |ui| {
                            gz_ui::text::label(ui, &text_, Font::Body, p.text, Clip::End);
                        })
                        .clicked()
                        {
                            go = i64::try_from(i).ok();
                        }
                    }
                });
            }
            if let Some(to) = go {
                ops.push(Op::Doc(DocOp::View(ViewOp::FindGo(to))));
            }
            return;
        }
        // the problems
        if self.data.problems.is_empty() {
            gz_ui::state::empty_state(
                &mut dui,
                Icon::Success,
                "No problems",
                "The graph is checked as it is edited.",
                None,
            );
            return;
        }
        let mut go: Option<(String, String)> = None;
        let all: String = self.data.problems.iter().map(|pr| format!("{}\r\n", pr.text())).collect();
        gz_ui::list::list(&mut dui, "gz-ui-graph.problems", vec2(0.0, -2.0), |l| {
            for (i, pr) in self.data.problems.iter().enumerate() {
                let selected = !pr.node.is_empty()
                    && self.mirror.selection.len() == 1
                    && self.mirror.selection.first().is_some_and(|s| *s == pr.node.as_str());
                let text_ = pr.text();
                let sentence = if pr.node.is_empty() {
                    pr.sentence()
                } else {
                    format!("{}: {}", pr.node, pr.sentence())
                };
                let resp = l.row(&format!("p{i}"), selected, |ui| {
                    let (icon, tone) = if pr.is_error() {
                        (Icon::Error, Tone::Error)
                    } else if pr.is_note() {
                        (Icon::Info, Tone::Muted)
                    } else {
                        (Icon::Warning, Tone::Warning)
                    };
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                    gz_ui::icons::draw(
                        ui.painter(),
                        icon,
                        rect,
                        if tone == Tone::Muted { p.text_muted } else { p.tone(tone) },
                    );
                    ui.add_space(6.0);
                    gz_ui::text::label(ui, &sentence, Font::Body, p.text, Clip::End);
                });
                let resp = resp.on_hover_text(format!("{}\nClick to frame the node", pr.code));
                // a message names a node, a slot and a parameter: it is meant to be pasted
                resp.context_menu(|ui| {
                    if menu_item(ui, "Copy", MenuItem::default()) {
                        ui.ctx().copy_text(text_.clone());
                    }
                    if menu_item(ui, "Copy All", MenuItem::default()) {
                        ui.ctx().copy_text(all.clone());
                    }
                });
                if resp.clicked() && !pr.node.is_empty() {
                    let slot = if pr.slot.starts_with("enter:") || pr.slot.starts_with("exit:") {
                        pr.slot.clone()
                    } else {
                        String::new()
                    };
                    go = Some((pr.node.clone(), slot));
                }
            }
        });
        if let Some((node, slot)) = go {
            let _ = graph;
            ops.push(Op::FrameNode(node));
            ops.push(Op::Slot(slot));
        }
    }

    // ---- the actions of the toolbar, the menu and the keys ------------------------------------

    /// Saves (`Save`) on the IO pool: what changed goes into the file's own text, over whatever
    /// the file took in meanwhile.
    pub fn save_now(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Some(doc) = self.shared() else { return Ok(()) };
        if !doc.path().inside {
            return Err(
                "the document is outside the project folder - it cannot be saved from here".to_owned()
            );
        }
        self.write(ctx, &doc);
        Ok(())
    }

    /// Takes the file's value of one conflict the banner lists.
    fn take_theirs(&mut self, ctx: &egui::Context, index: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let (ed, ctx2) = (self.editor.clone(), ctx.clone());
        let _task = self.editor.exec.spawn(async move {
            if let Ok(svc) = NqService::of(&ed)
                && let Err(e) = svc.take_theirs(&ed, &doc, Some(index)).await
            {
                ed.log.warn(format!("[nq] Take Theirs: {e}"));
            }
            ctx2.request_repaint();
        });
        self.refresher_force();
    }

    fn write(&mut self, ctx: &egui::Context, doc: &Arc<DocHandle>) {
        let (ed, d, ctx2) = (self.editor.clone(), doc.clone(), ctx.clone());
        let result: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
        let r2 = result.clone();
        self.set_message("Saving", Tone::Muted);
        let _task = self.editor.exec.spawn(async move {
            let r = match NqService::of(&ed) {
                Ok(svc) => svc.save(&ed, &d).await,
                Err(e) => Err(e.message),
            };
            let text_ = match &r {
                Ok(()) => "Saved".to_owned(),
                Err(e) => format!("Save failed: {e}"),
            };
            if r.is_err() {
                ed.log.warn(format!("[nq] {text_}"));
            }
            *r2.lock() = Some(r);
            if let Ok(q) = gz_app::UiQueue::of(&ed) {
                let id = doc_id(&d.path().wire, gz_nq::asset::is_behaviour_path(&d.path().wire));
                q.post(move |shell, _| {
                    if let Some(t) = shell
                        .document_mut(&id)
                        .and_then(|d| d.as_any_mut())
                        .and_then(|a| a.downcast_mut::<GraphTab>())
                    {
                        let tone = if text_.starts_with("Saved") {
                            Tone::Success
                        } else if text_.starts_with("Not saved") {
                            Tone::Warning
                        } else {
                            Tone::Error
                        };
                        t.set_message(&text_, tone);
                        t.refresher_force();
                    }
                });
            }
            ctx2.request_repaint();
        });
    }

    /// The next frame looks at the file again.
    fn refresher_force(&mut self) {
        self.force_refresh = true;
    }

    /// Reads the file again; asks first when there are unsaved edits, unless they are to be
    /// dropped (`discard`: `doc.reload discard=true` decided already).
    pub fn reload(&mut self, ctx: &egui::Context, discard: bool) {
        let Some(doc) = self.doc.clone() else { return };
        let run = {
            let (ed, d) = (self.editor.clone(), doc.clone());
            move || {
                let _task = ed.exec.clone().spawn(async move {
                    let r = match NqService::of(&ed) {
                        Ok(svc) => svc.reload(&ed, &d).await,
                        Err(e) => Err(e.message),
                    };
                    let text_ = match &r {
                        Ok(()) => "Reloaded".to_owned(),
                        Err(e) => format!("Reload failed: {e}"),
                    };
                    if let Ok(q) = gz_app::UiQueue::of(&ed) {
                        let id = doc_id(&d.path().wire, gz_nq::asset::is_behaviour_path(&d.path().wire));
                        q.post(move |shell, _| {
                            if let Some(t) = shell
                                .document_mut(&id)
                                .and_then(|d| d.as_any_mut())
                                .and_then(|a| a.downcast_mut::<GraphTab>())
                            {
                                t.set_message(
                                    &text_,
                                    if text_ == "Reloaded" { Tone::Success } else { Tone::Error },
                                );
                                t.refresher_force();
                            }
                        });
                    }
                });
            }
        };
        if !doc.is_dirty() || discard {
            run();
            return;
        }
        let spec = gz_ui::ConfirmSpec {
            id: "nq_reload".to_owned(),
            title: "Reload".to_owned(),
            headline: "Read the file again and drop the unsaved changes?".to_owned(),
            items: vec![self.wire.clone()],
            total: 1,
            consequence: "The unsaved changes are lost - this cannot be undone.".to_owned(),
            action: "Reload".to_owned(),
            destructive: true,
        };
        let asker = if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
        if let Some(line) = gz_ui::dialog::confirm(ctx, asker, spec, move |a| {
            if a == gz_ui::Answer::Yes {
                run();
            }
        }) {
            self.editor.log.info(line);
        }
    }

    /// Validate: the problems come up when there are any.
    pub fn validate(&mut self, ctx: &egui::Context) {
        let (e, w) = (self.data.errors, self.data.warnings);
        if e > 0 || w > 0 {
            self.set_drawer(ctx, DrawerTab::Problems);
        }
        let (text_, tone) = if e > 0 {
            ("Validated: errors found", Tone::Error)
        } else if w > 0 {
            ("Validated: warnings found", Tone::Warning)
        } else {
            ("Validated: no problems", Tone::Success)
        };
        self.set_message(text_, tone);
    }

    fn minimap_available(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| d.store.snapshot().graph.nodes.len() >= gz_nq::doc::MINIMAP_MIN_NODES)
    }

    fn minimap_tip(&self) -> String {
        let n = self.doc.as_ref().map_or(0, |d| d.store.snapshot().graph.nodes.len());
        if self.minimap_available() {
            "The whole graph in the canvas's corner: click or drag it to pan".to_owned()
        } else {
            format!(
                "A graph of {} nodes or more gets a minimap; this one has {n}",
                gz_nq::doc::MINIMAP_MIN_NODES
            )
        }
    }

    /// Runs `f` with the canvas and a context of its own, outside a frame (a command, a key, the
    /// Edit menu): what it asks for is queued for the next frame.
    pub fn with_canvas_cx<R>(
        &mut self,
        f: impl FnOnce(&mut Canvas, Option<&Scene>, &mut CanvasCx<'_>) -> R,
    ) -> Option<R> {
        let doc = self.doc.clone()?;
        let snap = doc.store.snapshot();
        let cat = self.data.catalog.clone()?;
        let scene = self.scene.clone();
        let mut ops = Vec::new();
        let mut actions = Vec::new();
        let r = {
            let mut ccx = CanvasCx {
                scene: scene.as_deref(),
                graph: &snap.graph,
                catalog: &cat,
                data: &self.data,
                view: &mut self.mirror,
                minimap: snap.view.minimap,
                ops: &mut ops,
                actions: &mut actions,
                editor: Some(&self.editor),
                prefs: self.prefs,
                edit: snap.edit,
                doc_status: None,
                covered: &[],
                pick_memory: &self.pick_memory,
            };
            f(&mut self.canvas, scene.as_deref(), &mut ccx)
        };
        self.queued.extend(ops);
        Some(r)
    }

    /// Pastes a fragment of nodes at a world point (or the pointer).
    fn paste_text(&mut self, text_: &str, at: Option<[f32; 2]>) {
        let t = text_.to_owned();
        self.with_canvas_cx(|c, _, ccx| c.paste(ccx, &t, at));
    }

    fn paste_from_clipboard(&mut self) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        let Some(text_) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else { return };
        self.paste_text(&text_, None);
    }

    /// The picker of the canvas (`graph.view menu=`): what it shows for, at the pointer or the
    /// canvas's centre.
    pub fn open_menu(&mut self, ctx: &egui::Context, name: &str) -> Result<(), String> {
        let (Some(doc), Some(cat), Some(sc)) =
            (self.doc.clone(), self.data.catalog.clone(), self.scene.clone())
        else {
            return Err("the graph is still being read".into());
        };
        let snap = doc.store.snapshot();
        let memory = self.pick_memory.clone();
        if name == "replace" {
            let Some(id) = self.mirror.selection.first().cloned() else {
                return Err("select the node to replace".into());
            };
            self.canvas.open_replace(ctx, &cat, &snap.graph, &self.mirror, &memory, &id);
            return Ok(());
        }
        if name == "picker" && self.canvas.hovered {
            self.canvas.open_picker_here(ctx, &cat, &snap.graph, &self.mirror, &memory);
            return Ok(());
        }
        self.canvas.open_menu_named(ctx, &sc, &cat, &snap.graph, &mut self.mirror, &memory, name)
    }

    /// The picker as a wire dragged out of an output or into an input puts it up.
    pub fn open_picker_wired(
        &mut self,
        ctx: &egui::Context,
        from: Option<(&str, &str)>,
        into: Option<&str>,
    ) -> Result<(), String> {
        let (Some(doc), Some(cat), Some(sc)) =
            (self.doc.clone(), self.data.catalog.clone(), self.scene.clone())
        else {
            return Err("the graph is still being read".into());
        };
        let snap = doc.store.snapshot();
        let memory = self.pick_memory.clone();
        self.canvas.open_picker_wired(ctx, &sc, &cat, &snap.graph, &self.mirror, &memory, from, into)
    }

    /// The wire under the pointer goes (the D key).
    pub fn remove_hovered_wire(&mut self, ctx: &egui::Context) -> bool {
        let c2 = ctx.clone();
        self.with_canvas_cx(|c, sc, ccx| sc.is_some_and(|sc| c.remove_hot_wire(&c2, sc, ccx)))
            .unwrap_or(false)
    }

    /// Opens Find in Project with the graph's search as its query.
    pub fn open_project_find(&mut self, ctx: &egui::Context) {
        let q = self.find_text.clone();
        self.project_find.open_with(&self.editor, ctx, &q);
    }

    /// The canvas's own view of what the doc says, for `graph.view`.
    #[must_use]
    pub fn wires_in_force(&self) -> WireSettings {
        self.doc.as_ref().map_or(self.wires, |d| d.store.snapshot().graph.canvas.wires.over(&self.wires))
    }
}

/// A key of the wire options a scene is built with.
fn wire_key(w: &WireSettings) -> u64 {
    let mut k = w.style as u64;
    k = k * 4 + w.diagonal as u64;
    k = k * 64 + (w.thickness * 10.0) as u64;
    for b in [w.by_pin, w.dim, w.bubbles, w.arrows] {
        k = k * 2 + u64::from(b);
    }
    k
}

impl Document for GraphTab {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        if self.behaviour { "beh" } else { "quest" }
    }

    fn label(&self) -> String {
        gz_nq::asset::base_name(&self.wire)
    }

    fn icon(&self) -> Icon {
        if self.behaviour { Icon::Behaviour } else { Icon::Quest }
    }

    fn path(&self) -> Option<String> {
        Some(self.wire.clone())
    }

    fn tooltip(&self) -> Option<String> {
        Some(self.wire.clone())
    }

    fn dirty(&self) -> bool {
        self.shared().is_some_and(|d| d.is_dirty())
    }

    fn dirty_elsewhere(&self) -> bool {
        true
    }

    fn missing(&self) -> bool {
        self.data.missing
    }

    fn padded(&self) -> bool {
        false
    }

    fn alive(&self) -> bool {
        // the document may have been closed without us (MCP quest_close): the tab follows
        let Some(doc) = &self.doc else { return true };
        let Some(session) = self.editor.project_opt() else { return false };
        NqService::get(&session).is_some_and(|svc| svc.docs().iter().any(|d| Arc::ptr_eq(d, doc)))
    }

    #[allow(clippy::too_many_lines)]
    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let ctx = cx.ctx.clone();
        let behaviour = self.behaviour;
        let (undo_n, redo_n) = self.doc.as_ref().map_or((0, 0), |d| {
            let h = d.store.history();
            (h.undo.len(), h.redo.len())
        });
        #[derive(PartialEq)]
        enum A {
            Save,
            Undo,
            Redo,
            Problems,
            FrameAll,
            FrameSel,
            Find,
            Keys,
            ZoomIn,
            ZoomOut,
            Zoom(i32),
            Minimap,
            GraphPanel,
            Details,
        }
        let mut act: Vec<A> = Vec::new();
        let mut run: Vec<&'static str> = Vec::new();
        // what each key is bound to now: the author may have rebound any of them
        let key = |id: &str| gz_app::actions::keys_label(cx.editor, id);
        let tip = |what: &str, id: &str| {
            let k = key(id);
            if k.is_empty() { what.to_owned() } else { format!("{what} ({k})") }
        };
        let (k_frame_all, k_frame_sel) = (key("graph.frame_all"), key("graph.frame_selection"));
        let snap = self.doc.as_ref().map(|d| d.store.snapshot());
        let bookmarks: Vec<(String, String)> = snap
            .as_ref()
            .map(|s| s.graph.canvas.bookmarks.iter().map(|b| (b.name.clone(), b.node.to_string())).collect())
            .unwrap_or_default();
        let (back_n, fwd_n) = snap.as_ref().map_or((0, 0), |s| (s.view.back.len(), s.view.forward.len()));
        let flow = snap.as_ref().map_or(Flow::Right, |s| s.graph.flow());
        let own_style = snap.as_ref().and_then(|s| s.graph.canvas.wires.style);
        tb.group(10);
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            &tip(if behaviour { "Save the graph" } else { "Save the quest" }, "file.save"),
            true,
            false,
            Kind::Ghost,
        ) {
            act.push(A::Save);
        }
        tb.group(8);
        if tb.action("undo", Icon::Undo, "Undo", &tip("Undo", "edit.undo"), undo_n > 0, false, Kind::Ghost) {
            act.push(A::Undo);
        }
        if tb.action("redo", Icon::Redo, "Redo", &tip("Redo", "edit.redo"), redo_n > 0, false, Kind::Ghost) {
            act.push(A::Redo);
        }
        tb.group(7);
        // the graph is checked as it is edited: this is here only when there is something to look at
        let (e, w) = (self.data.errors, self.data.warnings);
        if e > 0 || w > 0 {
            let count = if e > 0 {
                look::plural(e, "error", "errors")
            } else {
                look::plural(w, "warning", "warnings")
            };
            if tb.action(
                "problems",
                if e > 0 { Icon::Error } else { Icon::Warning },
                &count,
                "List the problems",
                true,
                false,
                Kind::Ghost,
            ) {
                act.push(A::Problems);
            }
        }
        if tb.action(
            "frame",
            Icon::FrameAll,
            "Frame All",
            &tip("Show the whole graph", "graph.frame_all"),
            true,
            false,
            Kind::Ghost,
        ) {
            act.push(A::FrameAll);
        }
        tb.group(6);
        let find_open = self.drawer() == DrawerTab::Find;
        if tb.action(
            "find",
            Icon::Search,
            "Find",
            &tip("Find nodes in this graph", "graph.find"),
            true,
            find_open,
            Kind::Ghost,
        ) {
            act.push(A::Find);
        }
        tb.spring();
        tb.group(5);
        // the panels: quiet toggles
        let graph_shown = self.graph_on_screen();
        if tb.action(
            "graph_panel",
            Icon::SidebarLeft,
            "Graph Panel",
            if behaviour {
                "The graph's settings, roles and variables"
            } else {
                "The quest's settings, tasks and variables"
            },
            true,
            graph_shown,
            Kind::Ghost,
        ) {
            act.push(A::GraphPanel);
        }
        if tb.action(
            "details",
            Icon::SidebarRight,
            "Details Panel",
            "The selected node and its action",
            true,
            self.show_details,
            Kind::Ghost,
        ) {
            act.push(A::Details);
        }
        let minimap_on = self.minimap_available();
        let minimap_vis = snap.as_ref().is_some_and(|s| s.view.minimap);
        if tb.action(
            "minimap",
            Icon::Minimap,
            "Minimap",
            &self.minimap_tip(),
            minimap_on,
            minimap_on && minimap_vis,
            Kind::Ghost,
        ) {
            act.push(A::Minimap);
        }
        tb.group(4);
        // the label is the zoom itself, one width whatever the level: a figure space leads two digits
        let pct = (gz_nq::doc::zoom_of(self.mirror.zoom) * 100.0 + 0.5) as i32;
        let zoom = if pct < 10 {
            format!("\u{2007}\u{2007}{pct}%")
        } else if pct < 100 {
            format!("\u{2007}{pct}%")
        } else {
            format!("{pct}%")
        };
        let z = self.mirror.zoom;
        let has_sel = !self.mirror.selection.is_empty();
        tb.menu("zoom", Icon::None, &zoom, "Zoom", Some("graph_zoom"), true, |ui| {
            if menu_item(
                ui,
                "Zoom In",
                MenuItem { icon: Icon::ZoomIn, enabled: z + 1 < ZOOM.len() as i32, ..MenuItem::default() },
            ) {
                act.push(A::ZoomIn);
            }
            if menu_item(
                ui,
                "Zoom Out",
                MenuItem { icon: Icon::ZoomOut, enabled: z > 0, ..MenuItem::default() },
            ) {
                act.push(A::ZoomOut);
            }
            ui.separator();
            if menu_item(
                ui,
                "Frame All",
                MenuItem { icon: Icon::ZoomFit, keys: &k_frame_all, ..MenuItem::default() },
            ) {
                act.push(A::FrameAll);
            }
            if menu_item(
                ui,
                "Frame Selection",
                MenuItem {
                    icon: Icon::FrameSelection,
                    keys: &k_frame_sel,
                    enabled: has_sel,
                    ..MenuItem::default()
                },
            ) {
                act.push(A::FrameSel);
            }
            ui.separator();
            for i in (0..ZOOM.len() as i32).rev() {
                let level = format!("{}%", (gz_nq::doc::zoom_of(i) * 100.0 + 0.5) as i32);
                if menu_item(ui, &level, MenuItem { selected: i == z, ..MenuItem::default() }) {
                    act.push(A::Zoom(i));
                }
            }
        });
        tb.group(3);
        let show_keys = self.show_keys;
        let row = |ui: &mut Ui, id: &'static str, label: Option<&str>, run: &mut Vec<&'static str>| {
            let Some(actions) = gz_app::Actions::of(cx.editor) else { return };
            let Some(a) = actions.table.read().get(id).copied() else { return };
            let keys = actions.table.read().keys_label(id);
            let (on, checked) =
                actions.with_cx(cx.editor, |acx| ((a.enabled)(acx), a.checked.is_some_and(|f| f(acx))));
            let item = MenuItem {
                icon: a.icon.unwrap_or(Icon::None),
                keys: &keys,
                enabled: on.on(),
                selected: checked,
                tip: on.why(),
                ..MenuItem::default()
            };
            if menu_item(ui, label.unwrap_or(a.title), item) {
                run.push(id);
            }
        };
        tb.menu(
            "more",
            Icon::More,
            "More",
            &format!("Everything else of this {}", self.noun()),
            Some("graph_more"),
            true,
            |ui| {
                row(ui, "graph.reload", None, &mut run);
                row(ui, "graph.auto_layout", None, &mut run);
                gz_app::chrome::local_submenu(
                    ui,
                    "Flow Direction",
                    if flow == Flow::Right { Icon::ChevronRight } else { Icon::ChevronDown },
                    true,
                    |ui| {
                        row(ui, "graph.flow_right", None, &mut run);
                        row(ui, "graph.flow_down", None, &mut run);
                    },
                );
                gz_app::chrome::local_submenu(ui, "Wires", Icon::Flow, true, |ui| {
                    row(ui, "graph.wires_default", Some("As in Preferences"), &mut run);
                    for (id, style) in [
                        ("graph.wires_curved", WireStyle::Curved),
                        ("graph.wires_straight", WireStyle::Straight),
                        ("graph.wires_circuit", WireStyle::Circuit),
                        ("graph.wires_angled", WireStyle::Angled),
                        ("graph.wires_subway", WireStyle::Subway),
                    ] {
                        let _ = own_style == Some(style);
                        row(ui, id, Some(style.title()), &mut run);
                    }
                });
                row(ui, "graph.find_in_project", None, &mut run);
                ui.separator();
                if menu_item(
                    ui,
                    "Back",
                    MenuItem {
                        icon: Icon::Back,
                        keys: &key("graph.back"),
                        enabled: back_n > 0,
                        ..MenuItem::default()
                    },
                ) {
                    run.push("graph.back");
                }
                if menu_item(
                    ui,
                    "Forward",
                    MenuItem {
                        icon: Icon::Forward,
                        keys: &key("graph.forward"),
                        enabled: fwd_n > 0,
                        ..MenuItem::default()
                    },
                ) {
                    run.push("graph.forward");
                }
                gz_app::chrome::local_submenu(ui, "Bookmarks", Icon::Bookmark, true, |ui| {
                    for (i, (name, node)) in bookmarks.iter().enumerate() {
                        let keys =
                            if i < 9 { key(&format!("graph.bookmark_{}", i + 1)) } else { String::new() };
                        if menu_item(
                            ui,
                            &format!("{name}  \u{b7}  {node}"),
                            MenuItem { keys: &keys, ..MenuItem::default() },
                        ) {
                            act.push(A::Zoom(-1 - i as i32));
                        }
                    }
                    if bookmarks.is_empty() {
                        menu_item(ui, "No bookmarks yet", MenuItem { enabled: false, ..MenuItem::default() });
                    }
                    ui.separator();
                    row(ui, "graph.bookmark", None, &mut run);
                    row(ui, "graph.previous_bookmark", None, &mut run);
                    row(ui, "graph.next_bookmark", None, &mut run);
                });
                if menu_item(
                    ui,
                    "Keyboard Shortcuts",
                    MenuItem { icon: Icon::Keyboard, selected: show_keys, ..MenuItem::default() },
                ) {
                    act.push(A::Keys);
                }
            },
        );

        for a in act {
            match a {
                A::Save => {
                    if let Err(e) = self.save_now(&ctx) {
                        self.set_message(&format!("Save failed: {e}"), Tone::Error);
                    }
                }
                A::Undo => self.queued.push(Op::Undo),
                A::Redo => self.queued.push(Op::Redo),
                A::Problems => self.set_drawer(&ctx, DrawerTab::Problems),
                A::FrameAll => self.canvas.request_frame_all(),
                A::FrameSel => self.canvas.request_frame_selection(),
                A::Find => self.open_find(&ctx),
                A::Keys => self.show_keys = !self.show_keys,
                A::ZoomIn => self.mirror.zoom = (self.mirror.zoom + 1).min(ZOOM.len() as i32 - 1),
                A::ZoomOut => self.mirror.zoom = (self.mirror.zoom - 1).max(0),
                // a bookmark of the list, by its place
                A::Zoom(i) if i < 0 => {
                    if let Some((name, _)) = bookmarks.get((-1 - i) as usize) {
                        self.queued.push(Op::Doc(DocOp::View(ViewOp::BookmarkGo(name.clone()))));
                    }
                }
                A::Zoom(i) => self.mirror.zoom = i,
                A::Minimap => self.queued.push(Op::Doc(DocOp::View(ViewOp::Minimap(!minimap_vis)))),
                A::GraphPanel => self.set_graph_panel(!graph_shown),
                A::Details => {
                    if self.details_rail && self.show_details {
                        self.details_overlay = !self.details_overlay;
                    } else {
                        self.show_details = !self.show_details;
                    }
                }
            }
        }
        for id in run {
            cx.requests.action(id);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        self.draw_body(ui, cx);
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let p = palette(ui.ctx());
        let n = self.doc.as_ref().map_or(0, |d| d.store.snapshot().graph.nodes.len());
        let text_ = format!(
            "{}  \u{b7}  {}  \u{b7}  {}",
            look::plural(n, "node", "nodes"),
            look::plural(self.data.errors, "error", "errors"),
            look::plural(self.data.warnings, "warning", "warnings")
        );
        let tip = self
            .data
            .catalog
            .as_ref()
            .map(|c| format!("Catalog: {}, version {}", c.source, c.header.version))
            .unwrap_or_default();
        let r = gz_ui::text::label(ui, &text_, Font::Small, p.text_secondary, Clip::End);
        if !tip.is_empty() {
            r.on_hover_text(tip);
        }
        if !self.message.is_empty() {
            ui.add_space(12.0);
            let c = if self.message_tone == Tone::Muted { p.text_muted } else { p.tone(self.message_tone) };
            gz_ui::text::label(ui, &self.message, Font::Small, c, Clip::End);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn menus(&mut self, bar: &mut gz_app::chrome::MenuBar<'_, '_>, cx: &mut DocCx<'_>) {
        let ctx = cx.ctx.clone();
        let mut act: Vec<&'static str> = Vec::new();
        let graph = self.graph_on_screen();
        let details = self.show_details;
        let problems = self.drawer() == DrawerTab::Problems;
        let mm_avail = self.minimap_available();
        let mm = mm_avail && self.doc.as_ref().is_some_and(|d| d.store.snapshot().view.minimap);
        let keys = self.show_keys;
        let own =
            self.doc.as_ref().map(|d| d.store.snapshot().graph.canvas.wires.clone()).unwrap_or_default();
        let set = own.over(&self.wires);
        let mut wire_ops: Vec<(&'static str, bool)> = Vec::new();
        bar.menu("Graph", |ui| {
            let row = |ui: &mut Ui, cx: &mut DocCx<'_>, id: &str| {
                gz_app::menu_row(ui, cx, id, None);
            };
            row(ui, cx, "file.save");
            row(ui, cx, "graph.reload");
            ui.separator();
            row(ui, cx, "graph.add_node");
            row(ui, cx, "graph.problems");
            ui.separator();
            gz_app::chrome::local_submenu(ui, "Flow Direction", Icon::Flow, true, |ui| {
                row(ui, cx, "graph.flow_right");
                row(ui, cx, "graph.flow_down");
            });
            gz_app::chrome::local_submenu(ui, "Wires", Icon::Flow, true, |ui| {
                gz_app::menu_row(ui, cx, "graph.wires_default", Some("As in Preferences"));
                ui.separator();
                for (id, style) in [
                    ("graph.wires_curved", WireStyle::Curved),
                    ("graph.wires_straight", WireStyle::Straight),
                    ("graph.wires_circuit", WireStyle::Circuit),
                    ("graph.wires_angled", WireStyle::Angled),
                    ("graph.wires_subway", WireStyle::Subway),
                ] {
                    gz_app::menu_row(ui, cx, id, Some(style.title()));
                }
                ui.separator();
                for (key, label, on) in [
                    ("by_pin", "Colour by Pin", set.by_pin),
                    ("dim", "Dim Unrelated", set.dim),
                    ("bubbles", "Flow Bubbles", set.bubbles),
                    ("arrows", "Arrowheads", set.arrows),
                ] {
                    if menu_item(ui, label, MenuItem { selected: on, ..MenuItem::default() }) {
                        wire_ops.push((key, !on));
                    }
                }
            });
            ui.separator();
            row(ui, cx, "graph.auto_layout");
            row(ui, cx, "graph.auto_layout_selection");
            row(ui, cx, "graph.auto_layout_branch");
            gz_app::chrome::local_submenu(ui, "Align", Icon::AutoLayout, true, |ui| {
                for id in [
                    "graph.align_left",
                    "graph.align_centre",
                    "graph.align_right",
                    "graph.align_top",
                    "graph.align_middle",
                    "graph.align_bottom",
                ] {
                    row(ui, cx, id);
                }
                ui.separator();
                row(ui, cx, "graph.distribute_horizontally");
                row(ui, cx, "graph.distribute_vertically");
                row(ui, cx, "graph.straighten");
            });
            gz_app::chrome::local_submenu(ui, "Select", Icon::Check, true, |ui| {
                row(ui, cx, "graph.select_all");
                row(ui, cx, "graph.select_connected");
                row(ui, cx, "graph.select_upstream");
                row(ui, cx, "graph.select_downstream");
            });
            gz_app::chrome::local_submenu(ui, "Node", Icon::Quest, true, |ui| {
                row(ui, cx, "graph.rename");
                row(ui, cx, "graph.replace_node");
                row(ui, cx, "graph.duplicate_with_wires");
                row(ui, cx, "graph.disable");
                row(ui, cx, "graph.collapse");
                row(ui, cx, "graph.add_frame");
                row(ui, cx, "graph.connect_nearest");
                row(ui, cx, "graph.bypass");
            });
            ui.separator();
            row(ui, cx, "graph.frame_all");
            row(ui, cx, "graph.frame_selection");
            row(ui, cx, "graph.find");
            row(ui, cx, "graph.find_next");
            row(ui, cx, "graph.find_previous");
            row(ui, cx, "graph.find_in_project");
            ui.separator();
            gz_app::chrome::local_submenu(ui, "Bookmarks", Icon::Bookmark, true, |ui| {
                row(ui, cx, "graph.bookmark");
                row(ui, cx, "graph.previous_bookmark");
                row(ui, cx, "graph.next_bookmark");
            });
            ui.separator();
            let item = |ui: &mut Ui, label: &str, selected: bool, enabled: bool| {
                menu_item(ui, label, MenuItem { selected, enabled, ..MenuItem::default() })
            };
            if item(ui, "Graph Panel", graph, true) {
                act.push("graph_panel");
            }
            if item(ui, "Details Panel", details, true) {
                act.push("details");
            }
            if item(ui, "Problems", problems, true) {
                act.push("problems");
            }
            if item(ui, "Minimap", mm, mm_avail) {
                act.push("minimap");
            }
            if item(ui, "Keyboard Shortcuts", keys, true) {
                act.push("keys");
            }
        });
        for (key, on) in wire_ops {
            let mut op = gz_luadata::Value::table();
            op.set("op", "wires");
            op.set(key, on);
            self.queued.push(Op::Doc(DocOp::Apply(op)));
        }
        for a in act {
            match a {
                "graph_panel" => self.set_graph_panel(!graph),
                "details" => self.show_details = !self.show_details,
                "problems" => {
                    self.set_drawer(&ctx, if problems { DrawerTab::None } else { DrawerTab::Problems })
                }
                "minimap" => self.queued.push(Op::Doc(DocOp::View(ViewOp::Minimap(!mm)))),
                "keys" => self.show_keys = !self.show_keys,
                _ => {}
            }
        }
    }

    /// What has the keys: the canvas, while the pointer is on it or it has the focus. The graph's
    /// actions of the canvas take their keys then.
    fn key_part(&self, ctx: &egui::Context) -> Option<&'static str> {
        self.canvas.takes_keys(ctx).then_some("canvas")
    }

    /// Esc closes the shortcuts card and drops a gesture; every other key is an action's.
    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        let plain = !chord.ctrl && !chord.shift && !chord.alt;
        if chord.key == Key::Escape && plain && self.show_keys {
            self.show_keys = false;
            return true;
        }
        if chord.key == Key::Escape && plain && self.canvas.takes_keys(cx.ctx) {
            return self.canvas.escape();
        }
        false
    }

    fn reload(&mut self, cx: &mut DocCx<'_>, discard: bool) -> Result<(), String> {
        let ctx = cx.ctx.clone();
        GraphTab::reload(self, &ctx, discard);
        Ok(())
    }

    fn edit(&mut self, cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        if !run {
            return true;
        }
        let ctx = cx.ctx.clone();
        match action {
            EditAction::Undo => self.queued.push(Op::Undo),
            EditAction::Redo => self.queued.push(Op::Redo),
            EditAction::Copy | EditAction::Cut => {
                let graph = self.doc.as_ref().map(|d| d.store.snapshot());
                if let Some(snap) = graph
                    && let Some(t) = Canvas::copy_text(&snap.graph, &self.mirror.selection)
                {
                    ctx.copy_text(t);
                }
                if action == EditAction::Cut && !self.mirror.selection.is_empty() {
                    self.queued.push(Op::Doc(DocOp::Canvas(CanvasOp::Delete)));
                }
            }
            EditAction::Paste => self.paste_from_clipboard(),
            EditAction::Duplicate => {
                self.queued.push(Op::Doc(DocOp::Canvas(CanvasOp::Duplicate { wires: false })))
            }
            EditAction::Delete => {
                if !self.mirror.selection.is_empty() {
                    self.queued.push(Op::Doc(DocOp::Canvas(CanvasOp::Delete)));
                }
            }
            EditAction::Find => self.open_find(&ctx),
        }
        true
    }

    fn save(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        let ctx = cx.ctx.clone();
        self.save_now(&ctx)
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        // Don't Save puts the file's content back
        if let Some(doc) = self.shared() {
            let ed = self.editor.clone();
            let _task = self.editor.exec.spawn(async move {
                if let Ok(svc) = NqService::of(&ed) {
                    let _ = svc.reload(&ed, &doc).await;
                }
            });
        }
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        if let Some(session) = self.editor.project_opt()
            && let Some(svc) = NqService::get(&session)
        {
            let _ = svc.close(&self.wire, true);
        }
    }

    fn restore_arg(&self) -> Option<String> {
        Some(self.wire.clone())
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.wire,
            "graph_panel": self.graph_panel_state(),
            "details": self.show_details,
            "selected": self.mirror.selection,
            "slot": self.mirror.slot,
            "zoom_level": self.mirror.zoom,
        })
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// A text of a toolbar row the kit measures.
#[must_use]
pub fn status_line(n: usize, e: usize, w: usize) -> String {
    format!(
        "{}  \u{b7}  {}  \u{b7}  {}",
        look::plural(n, "node", "nodes"),
        look::plural(e, "error", "errors"),
        look::plural(w, "warning", "warnings")
    )
}

/// The Keyboard Shortcuts card as a frame laid it out: where it is, and whether its cross was clicked.
struct KeysCard {
    rect: Option<Rect>,
    close: bool,
}

/// The Keyboard Shortcuts card over the canvas `c`, `last` its rect in the frame before, `salt` its tab. It
/// guards the canvas under it - a click or a drag on it is the card's - and goes in after the canvas.
fn keys_card(
    ui: &mut Ui,
    c: Rect,
    last: Option<Rect>,
    salt: &str,
    keys_of: &dyn Fn(&str) -> String,
) -> KeysCard {
    let w = 440.0f32.min(c.width() - 16.0);
    if w < 160.0 {
        return KeysCard { rect: None, close: false };
    }
    let p = palette(ui.ctx());
    let r = Rect::from_min_max(pos2(c.max.x - w - 8.0, c.min.y + 8.0), pos2(c.max.x - 8.0, c.max.y - 8.0));
    // the canvas under the card takes no click meant for it: the guard goes in UNDER the card's
    // own controls (registered first), or it would take their clicks too - the cross among them.
    // The card's size is the last frame's; its first frame guards the room it may take.
    ui.interact(
        last.unwrap_or(r),
        egui::Id::unique(("gz-ui-graph.keys_block", salt)),
        egui::Sense::click_and_drag(),
    );
    let mut close = false;
    let mut kui = ui.new_child(UiBuilder::new().id_salt(("gz-ui-graph.keys", salt)).max_rect(r));
    kui.set_clip_rect(r.intersect(ui.clip_rect()));
    let frame = egui::Frame::new()
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8));
    let inner = frame.show(&mut kui, |ui| {
        ui.set_width(w - 16.0);
        ui.horizontal(|ui| {
            gz_ui::text::label(ui, "Keyboard Shortcuts", Font::Strong, p.text, Clip::End);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if gz_ui::icon_button(ui, Icon::Close, "Close (Esc)", false, true).clicked() {
                    close = true;
                }
            });
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "They work while the cursor is over the graph. Every edit is also an MCP call.",
                )
                .font(Font::Small.id())
                .color(p.text_muted),
            )
            .wrap(),
        );
        egui::ScrollArea::vertical()
            .id_salt("gz-ui-graph.keys_rows")
            .max_height((r.height() - 80.0).max(60.0))
            .show(ui, |ui| {
                for (row, what) in &KEYS {
                    match row {
                        KeyRow::Head => {
                            ui.add_space(2.0);
                            gz_ui::text::label(ui, what, Font::Strong, p.text, Clip::End);
                        }
                        _ => {
                            let k = key_text(row, keys_of);
                            ui.horizontal(|ui| {
                                ui.scope(|ui| {
                                    ui.set_width(gz_ui::units::em(9.0));
                                    gz_ui::text::label(ui, &k, Font::Body, p.text, Clip::End);
                                });
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(*what)
                                            .font(Font::Body.id())
                                            .color(p.text_secondary),
                                    )
                                    .wrap(),
                                );
                            });
                        }
                    }
                }
            });
    });
    KeysCard { rect: Some(inner.response.rect), close }
}

/// One conflict as the banner lists it: where, and what each side says.
fn conflict_line(c: &gz_luadata::Conflict) -> String {
    let show = |v: &Option<gz_luadata::Value>| match v {
        None => "removed".to_owned(),
        Some(v) => gz_luadata::inline(v).unwrap_or_else(|| "a long text".to_owned()),
    };
    let line = c.span.map_or_else(String::new, |s| format!(" (line {})", s.line));
    format!("{}{line}: here {}, on disk {}", gz_luadata::path::show(&c.path), show(&c.mine), show(&c.theirs))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Sense, pos2, vec2};

    /// One frame: a stand-in for the canvas, then the card over it, as the tab draws them.
    fn frame(
        ctx: &Context,
        t: f64,
        events: Vec<Event>,
        last: Option<Rect>,
        canvas_clicks: &mut u32,
    ) -> super::KeysCard {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0));
        let mut card = None;
        let mut out = ctx.run_ui(
            RawInput { screen_rect: Some(screen), time: Some(t), events, ..RawInput::default() },
            |ui| {
                let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(1200.0, 800.0));
                if ui
                    .interact(canvas, egui::Id::unique("gz-ui-graph.test_canvas"), Sense::click_and_drag())
                    .clicked()
                {
                    *canvas_clicks += 1;
                }
                card = Some(super::keys_card(ui, canvas, last, "t", &|_| String::new()));
            },
        );
        // the textures egui made are the painter's, and here nobody paints
        out.textures_delta.clear();
        card.unwrap()
    }

    fn click(ctx: &Context, t: &mut f64, at: Pos2, last: Option<Rect>, canvas_clicks: &mut u32) -> bool {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut closed = false;
        for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]] {
            *t += 0.05;
            closed |= frame(ctx, *t, events, last, canvas_clicks).close;
        }
        closed
    }

    /// The card's cross closes it, and a click on the card is not the canvas's: the guard against
    /// the canvas lies under the card's controls (it once lay over them and took the cross's click).
    #[test]
    fn the_cross_closes_the_card_and_the_canvas_under_it_takes_no_click() {
        let ctx = Context::default();
        gz_ui::font::install(&ctx, gz_ui::font::FontFiles::default());
        let mut canvas_clicks = 0;
        let mut t = 0.0;
        let mut last = None;
        for _ in 0..2 {
            t += 0.05;
            last = frame(&ctx, t, vec![], last, &mut canvas_clicks).rect;
        }
        let card = last.unwrap();

        // the card's body: neither a close nor a click of the canvas
        let body = pos2(card.center().x, card.max.y - 20.0);
        assert!(!click(&ctx, &mut t, body, last, &mut canvas_clicks));
        assert_eq!(canvas_clicks, 0, "a click on the card reached the canvas");

        // the canvas beside the card still takes its clicks
        assert!(!click(&ctx, &mut t, pos2(40.0, 400.0), last, &mut canvas_clicks));
        assert_eq!(canvas_clicks, 1);

        // the cross, at the card's top right
        let cross = pos2(card.max.x - 8.0 - 10.0, card.min.y + 8.0 + 10.0);
        assert!(click(&ctx, &mut t, cross, last, &mut canvas_clicks), "the cross did not close the card");
        assert_eq!(canvas_clicks, 1);
    }
}
