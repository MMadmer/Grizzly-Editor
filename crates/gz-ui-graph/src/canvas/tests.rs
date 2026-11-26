#![allow(clippy::unwrap_used)]

use egui::{Context, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use gz_nq::canvas::WireSettings;
use gz_nq::catalog::{Kind, USE_EXTRA, USE_MAIN, USE_TRIGGER};
use gz_nq::doc::DocOp;
use gz_nq::{Catalog, Node, asset::Graph};

use super::{
    Canvas, CanvasCx, CanvasPrefs, Detail, FIXED_TITLE_PX, MIN_TEXT_PX, Op, PickWant, ViewMirror, pick_kinds,
};
use crate::derived::DocData;
use crate::scene::{Scene, SceneInput};

fn catalog() -> Catalog {
    let kind = |id: &str, uses: u8, pins: &[&str]| Kind {
        id: id.into(),
        title: id.into(),
        uses,
        pins: pins.iter().map(|p| (*p).to_owned()).collect(),
        in_quest: true,
        in_behaviour: true,
        ..Kind::default()
    };
    Catalog::from_kinds(vec![
        kind("trigger.start", USE_TRIGGER, &["next"]),
        kind("flow.step", USE_MAIN, &["next"]),
        kind("flow.end", USE_MAIN, &[]),
        kind("wait.when", USE_MAIN, &["done", "timeout"]),
        kind("var.set", USE_EXTRA, &[]),
    ])
}

fn ids(kinds: &[&Kind]) -> Vec<String> {
    let mut v: Vec<String> = kinds.iter().map(|k| k.id.clone()).collect();
    v.sort();
    v
}

/// From an output only kinds that take an input; into an input only kinds that go on; on a
/// wire both; in an action slot only actions.
#[test]
fn the_picker_offers_what_the_pin_takes() {
    let cat = catalog();
    assert_eq!(
        ids(&pick_kinds(&cat, false, PickWant::Any)),
        ["flow.end", "flow.step", "trigger.start", "wait.when"]
    );
    assert_eq!(ids(&pick_kinds(&cat, false, PickWant::TakesInput)), ["flow.end", "flow.step", "wait.when"]);
    assert_eq!(ids(&pick_kinds(&cat, false, PickWant::GoesOn)), ["flow.step", "trigger.start", "wait.when"]);
    assert_eq!(ids(&pick_kinds(&cat, false, PickWant::OnWire)), ["flow.step", "wait.when"]);
    assert_eq!(ids(&pick_kinds(&cat, false, PickWant::Action)), ["var.set"]);
    assert_eq!(ids(&pick_kinds(&cat, false, PickWant::Trigger)), ["trigger.start"]);
}

/// Text is never under 11 px: past that a node shows its title alone at a fixed size, and a node
/// too small even for that is a block.
#[test]
fn semantic_zoom_thresholds() {
    let at = |zoom: i32| ViewMirror { zoom, ..ViewMirror::default() };
    // 1:1 at 100 %: everything
    assert_eq!(Canvas::detail(&at(gz_nq::doc::ZOOM_DEFAULT), 1.0, 240.0), Detail::Full);
    // a body line of 15 pt is 11 px at 75 %, 9 px at 60 %
    let l75 = gz_nq::doc::ZOOM.iter().position(|z| (*z - 0.75).abs() < 1e-6).unwrap() as i32;
    assert_eq!(Canvas::detail(&at(l75), 1.0, 240.0), Detail::Full);
    assert_eq!(Canvas::detail(&at(l75 - 1), 1.0, 240.0), Detail::Header);
    // a UI scale of 1.5 keeps the body readable further out
    assert_eq!(Canvas::detail(&at(l75 - 1), 1.5, 240.0), Detail::Full);
    // the title of a header-only node keeps its size
    assert!((Canvas::title_px(&at(l75 - 3), 1.0) - FIXED_TITLE_PX).abs() < f32::EPSILON);
    assert!(Canvas::title_px(&at(gz_nq::doc::ZOOM_DEFAULT), 1.0) >= MIN_TEXT_PX);
    // far out a node is a block
    assert_eq!(Canvas::detail(&at(0), 1.0, 240.0), Detail::Block);
    // every level that draws text draws it at 11 px or more
    for z in 0..gz_nq::doc::ZOOM.len() as i32 {
        let v = at(z);
        if Canvas::detail(&v, 1.0, 180.0) != Detail::Block {
            assert!(Canvas::title_px(&v, 1.0) >= MIN_TEXT_PX, "level {z}");
        }
    }
}

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
    let p = gz_ui::Palette::default();
    let scene = Scene::build(&SceneInput {
        graph: &g,
        catalog: &cat,
        edit: 1,
        palette: &p,
        wires: WireSettings::default(),
        names: None,
        lang: None,
    });
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
            let mut actions = Vec::new();
            let mut out = ctx.run_ui(
                RawInput { screen_rect: Some(rect), time: Some(t), events, ..RawInput::default() },
                |ui| {
                    let mut cx = CanvasCx {
                        scene: Some(&scene),
                        graph: &g,
                        catalog: &cat,
                        data: &data,
                        view,
                        minimap: false,
                        ops: &mut ops,
                        actions: &mut actions,
                        editor: None,
                        prefs: CanvasPrefs::default(),
                        edit: 1,
                        doc_status: None,
                        covered: &[card],
                        pick_memory: "graph.test",
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

/// A canvas over a scene, driven by input events as the window sends them.
struct Rig {
    ctx: Context,
    g: Graph,
    cat: Catalog,
    scene: Scene,
    data: DocData,
    view: ViewMirror,
    canvas: Canvas,
    t: f64,
    rect: Rect,
    /// The keys held in the frames to come, and as the context last heard of them.
    mods: Modifiers,
    told: Modifiers,
}

impl Rig {
    /// `s` (a trigger) -> `a` -> `b` across, the wire `a` -> `b` with two reroutes.
    fn new() -> Self {
        let ctx = Context::default();
        gz_ui::font::install(&ctx, gz_ui::font::FontFiles::default());
        let mut g = Graph::default();
        let mut s = Node::new("s", "trigger.start");
        s.connect("next", "a");
        s.pos = [-420.0, -40.0];
        let mut a = Node::new("a", "flow.step");
        a.connect("next", "b");
        a.pos = [-120.0, -40.0];
        let mut b = Node::new("b", "flow.step");
        b.pos = [260.0, -40.0];
        for mut n in [s, a, b] {
            n.has_pos = true;
            g.nodes.push_back(n);
        }
        g.canvas.set_route("a", "next", "b", vec![[140.0, 150.0], [200.0, 150.0]]);
        let cat = catalog();
        let mut rig = Self {
            ctx,
            scene: Scene::build(&SceneInput {
                graph: &g,
                catalog: &cat,
                edit: 1,
                palette: &gz_ui::Palette::default(),
                wires: WireSettings::default(),
                names: None,
                lang: None,
            }),
            g,
            cat,
            data: DocData::default(),
            view: ViewMirror::default(),
            canvas: Canvas::new("rig", false),
            t: 0.0,
            rect: Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)),
            mods: Modifiers::NONE,
            told: Modifiers::NONE,
        };
        rig.frame(vec![]);
        rig
    }

    /// One frame with `events`; what the canvas asked of the tab.
    fn frame(&mut self, mut events: Vec<Event>) -> Vec<Op> {
        self.t += 0.05;
        if self.mods != self.told {
            events.insert(0, Event::ModifiersChanged(self.mods));
            self.told = self.mods;
        }
        let mut ops = Vec::new();
        let mut actions = Vec::new();
        let (scene, g, cat, data, view, canvas, rect) =
            (&self.scene, &self.g, &self.cat, &self.data, &mut self.view, &mut self.canvas, self.rect);
        let mut out = self.ctx.run_ui(
            RawInput { screen_rect: Some(rect), time: Some(self.t), events, ..RawInput::default() },
            |ui| {
                let mut cx = CanvasCx {
                    scene: Some(scene),
                    graph: g,
                    catalog: cat,
                    data,
                    view,
                    minimap: false,
                    ops: &mut ops,
                    actions: &mut actions,
                    editor: None,
                    prefs: CanvasPrefs::default(),
                    edit: 1,
                    doc_status: None,
                    covered: &[],
                    pick_memory: "graph.test",
                };
                canvas.draw(ui, rect, &mut cx);
            },
        );
        out.textures_delta.clear();
        ops
    }

    /// The scene made again from the graph, as the tab does after an edit.
    fn rebuild(&mut self) {
        self.scene = Scene::build(&SceneInput {
            graph: &self.g,
            catalog: &self.cat,
            edit: self.scene.edit + 1,
            palette: &gz_ui::Palette::default(),
            wires: WireSettings::default(),
            names: None,
            lang: None,
        });
        self.frame(vec![]);
    }

    fn screen(&self, w: [f32; 2]) -> Pos2 {
        self.canvas.to_screen(&self.view, w)
    }

    fn button(at: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }
    }

    /// A click (or, `twice`, a double click) at `at`: every op it asked for.
    fn click(&mut self, at: Pos2, modifiers: Modifiers, twice: bool) -> Vec<Op> {
        // a gesture of its own: no double click with the one before
        self.t += 1.0;
        self.mods = modifiers;
        let mut ops = self.frame(vec![Event::PointerMoved(at)]);
        for _ in 0..if twice { 2 } else { 1 } {
            ops.extend(self.frame(vec![Self::button(at, true, modifiers)]));
            ops.extend(self.frame(vec![Self::button(at, false, modifiers)]));
        }
        ops.extend(self.frame(vec![]));
        self.mods = Modifiers::NONE;
        ops
    }

    /// A drag from `a` to `b` with the left button: every op it asked for.
    fn drag(&mut self, a: Pos2, b: Pos2, modifiers: Modifiers) -> Vec<Op> {
        self.t += 1.0;
        self.mods = modifiers;
        let mut ops = self.frame(vec![Event::PointerMoved(a)]);
        ops.extend(self.frame(vec![Self::button(a, true, modifiers)]));
        for k in 1..=6 {
            let p = a + (b - a) * (k as f32 / 6.0);
            ops.extend(self.frame(vec![Event::PointerMoved(p)]));
        }
        ops.extend(self.frame(vec![Self::button(b, false, modifiers)]));
        ops.extend(self.frame(vec![]));
        self.mods = Modifiers::NONE;
        ops
    }
}

/// The ops of a batch or of one op, flattened.
fn doc_ops(ops: &[Op]) -> Vec<gz_luadata::Value> {
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::Doc(DocOp::Apply(v)) => out.push(v.clone()),
            Op::Doc(DocOp::ApplyOps(v)) => out.extend(v.arr().iter().cloned()),
            _ => {}
        }
    }
    out
}

fn named<'a>(ops: &'a [gz_luadata::Value], name: &str) -> Vec<&'a gz_luadata::Value> {
    ops.iter().filter(|o| o.get_string("op", "") == name).collect()
}

/// A double click on a wire puts a reroute there - told at the second press, as the author
/// feels it - and a double click on a reroute takes it away.
#[test]
fn a_double_click_on_a_wire_adds_a_reroute_and_on_a_reroute_removes_it() {
    let mut rig = Rig::new();
    // the middle of the wire s -> a
    let li = rig.scene.links.iter().position(|l| l.via.is_empty()).unwrap();
    let pts = rig.scene.links[li].points.clone();
    let mid = gz_nq::wires::point_at(&pts, 0.5).map(|(q, _)| q).unwrap();
    let ops = doc_ops(&rig.click(rig.screen(mid), Modifiers::NONE, true));
    let routes = named(&ops, "route");
    assert_eq!(routes.len(), 1, "{ops:?}");
    assert_eq!(routes[0].get_string("from", ""), "s");
    assert_eq!(routes[0].get("points").map_or(0, |p| p.arr().len()), 1);
    // one click on a wire adds nothing
    let ops = doc_ops(&rig.click(rig.screen(mid), Modifiers::NONE, false));
    assert!(named(&ops, "route").is_empty(), "{ops:?}");
    // a double click on the first reroute of a -> b leaves the second
    let ops = doc_ops(&rig.click(rig.screen([140.0, 150.0]), Modifiers::NONE, true));
    let routes = named(&ops, "route");
    assert_eq!(routes.len(), 1, "{ops:?}");
    let left: Vec<f64> = routes[0]
        .get("points")
        .map(|p| {
            p.arr()
                .iter()
                .flat_map(|q| q.arr().iter().map(|v| v.to_number(0.0)).collect::<Vec<_>>())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(left, vec![200.0, 150.0]);
}

/// A double click on a node's header renames it: the Details panel focuses the id - once the
/// button is let go, or the release would take the focus away again.
#[test]
fn a_double_click_on_a_header_renames_the_node() {
    let mut rig = Rig::new();
    let n = rig.scene.node_of("a").unwrap();
    let r = rig.scene.nodes[n].rect();
    let at = rig.screen([r[0] + 20.0, r[1] + 8.0]);
    rig.t += 1.0;
    let _ = rig.frame(vec![Event::PointerMoved(at)]);
    let _ = rig.frame(vec![Rig::button(at, true, Modifiers::NONE)]);
    let _ = rig.frame(vec![Rig::button(at, false, Modifiers::NONE)]);
    let _ = rig.frame(vec![Rig::button(at, true, Modifiers::NONE)]);
    assert!(!rig.canvas.want_rename, "the rename was asked for while the button was down");
    let _ = rig.frame(vec![Rig::button(at, false, Modifiers::NONE)]);
    assert!(rig.canvas.want_rename);
    assert_eq!(rig.view.selection, vec![smol_str::SmolStr::new("a")]);
}

/// A selected reroute is the reroute at its point: one put in before it on its wire (an undo, the
/// file changed outside) does not hand the selection to its neighbour; a node selection set from
/// outside the canvas drops it; Delete removes the nodes first and leaves their wires alone.
#[test]
fn a_selected_reroute_keeps_its_identity() {
    let mut rig = Rig::new();
    let _ = rig.click(rig.screen([200.0, 150.0]), Modifiers::NONE, false);
    assert_eq!(rig.canvas.selected_knots(), 1);
    // a reroute put in at the front of the same wire
    rig.g.canvas.set_route("a", "next", "b", vec![[100.0, 150.0], [140.0, 150.0], [200.0, 150.0]]);
    rig.rebuild();
    let li = rig.scene.links.iter().position(|l| l.via.len() == 3).unwrap();
    assert_eq!(rig.canvas.knot_hits, vec![(li, 2)], "the selection moved to another reroute");
    // the point taken away: the selection goes with it
    rig.g.canvas.set_route("a", "next", "b", vec![[100.0, 150.0], [140.0, 150.0]]);
    rig.rebuild();
    assert_eq!(rig.canvas.selected_knots(), 0);
    // a node selection set from outside drops a selected reroute
    let _ = rig.click(rig.screen([140.0, 150.0]), Modifiers::NONE, false);
    assert_eq!(rig.canvas.selected_knots(), 1);
    rig.view.selection = vec![smol_str::SmolStr::new("s")];
    let _ = rig.frame(vec![]);
    assert_eq!(rig.canvas.selected_knots(), 0);
    // Delete with a node: the node first, and a wire of a node that goes is not routed again
    let _ = rig.click(rig.screen([140.0, 150.0]), Modifiers::NONE, false);
    let del = rig.canvas.delete_ops(&rig.scene, &[smol_str::SmolStr::new("b")]).unwrap();
    let ops = del.arr();
    assert_eq!(ops.len(), 1, "{ops:?}");
    assert_eq!(ops[0].get_string("op", ""), "remove");
}

/// Reroutes are selected like nodes: a click, Ctrl for more, the marquee; a drag moves every
/// selected node and reroute in one undo step; Delete takes the selected reroutes out.
#[test]
fn reroutes_are_selected_and_moved_with_the_nodes() {
    let mut rig = Rig::new();
    // a click on a reroute selects it alone
    let _ = rig.click(rig.screen([140.0, 150.0]), Modifiers::NONE, false);
    assert_eq!(rig.canvas.selected_knots(), 1);
    assert!(rig.view.selection.is_empty());
    // Ctrl adds a node
    let n = rig.scene.node_of("b").unwrap();
    let r = rig.scene.nodes[n].rect();
    let body = rig.screen([r[0] + 30.0, (r[1] + r[3]) * 0.5]);
    let _ = rig.click(body, Modifiers::CTRL, false);
    assert_eq!(rig.view.selection, vec![smol_str::SmolStr::new("b")]);
    assert_eq!(rig.canvas.selected_knots(), 1);
    // dragging the node moves it and the selected reroute, one batch
    let ops = rig.drag(body, body + vec2(48.0, 32.0), Modifiers::NONE);
    let batch: Vec<&Op> = ops.iter().filter(|o| matches!(o, Op::Doc(DocOp::ApplyOps(_)))).collect();
    assert_eq!(batch.len(), 1, "{ops:?}");
    let flat = doc_ops(&ops);
    let moves = named(&flat, "move");
    let routes = named(&flat, "route");
    assert_eq!((moves.len(), routes.len()), (1, 1), "{flat:?}");
    let first = routes[0]
        .get("points")
        .map(|p| p.arr()[0].arr().iter().map(|v| v.to_number(0.0)).collect::<Vec<_>>());
    let second = routes[0]
        .get("points")
        .map(|p| p.arr()[1].arr().iter().map(|v| v.to_number(0.0)).collect::<Vec<_>>());
    // the selected reroute went along, the other stayed
    assert_ne!(first, Some(vec![140.0, 150.0]), "{flat:?}");
    assert_eq!(second, Some(vec![200.0, 150.0]), "{flat:?}");
    // a click on empty space drops the selection, reroutes too
    let _ = rig.click(rig.screen([0.0, 320.0]), Modifiers::NONE, false);
    assert!(rig.view.selection.is_empty());
    assert_eq!(rig.canvas.selected_knots(), 0);
    // the marquee takes both reroutes
    let _ = rig.drag(rig.screen([100.0, 200.0]), rig.screen([240.0, 110.0]), Modifiers::NONE);
    assert_eq!(rig.canvas.selected_knots(), 2);
    // Delete: the wire keeps no reroute
    let del = rig.canvas.delete_ops(&rig.scene, &[]).unwrap();
    assert_eq!(del.arr().len(), 1);
    assert_eq!(del.arr()[0].get("points").map_or(9, |p| p.arr().len()), 0);
    assert_eq!(rig.canvas.selected_knots(), 0);
}

/// An action is moved to the other slot by hand: dropped just under a node it runs on that
/// node's exit, whether the node has actions on exit or not.
#[test]
fn an_action_dropped_under_a_node_runs_on_its_exit() {
    let mut rig = Rig::new();
    let a = rig.g.nodes.iter_mut().find(|n| n.id == "a").unwrap();
    a.on_enter.push(gz_nq::asset::Action { kind: "var.set".into(), params: gz_luadata::Value::table() });
    rig.rebuild();
    let n = rig.scene.node_of("a").unwrap();
    let node = &rig.scene.nodes[n];
    let r = node.rect();
    let row_y = r[1] + node.b.actions_y + gz_nq::layout::ACTION_ROW * 0.5;
    let from = rig.screen([r[0] + 40.0, row_y]);
    let b = rig.scene.node_of("b").unwrap();
    let rb = rig.scene.nodes[b].rect();
    let to = rig.screen([(rb[0] + rb[2]) * 0.5, rb[3] + 8.0]);
    let ops = doc_ops(&rig.drag(from, to, Modifiers::NONE));
    let moves = named(&ops, "move_action");
    assert_eq!(moves.len(), 1, "{ops:?}");
    let m = moves[0];
    assert_eq!(
        (
            m.get_string("id", ""),
            m.get_string("slot", ""),
            m.get_string("to_id", ""),
            m.get_string("to_slot", "")
        ),
        ("a".to_owned(), "enter".to_owned(), "b".to_owned(), "exit".to_owned()),
        "{m:?}"
    );
}
