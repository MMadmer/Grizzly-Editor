#![allow(clippy::unwrap_used)]

use egui::{Context, Event, Modifiers, MouseWheelUnit, Pos2, RawInput, Rect, pos2, vec2};
use gz_nq::canvas::WireSettings;
use gz_nq::catalog::{Kind, USE_EXTRA, USE_MAIN, USE_TRIGGER};
use gz_nq::{Catalog, Node, asset::Graph};

use super::{
    Canvas, CanvasCx, CanvasPrefs, Detail, FIXED_TITLE_PX, MIN_TEXT_PX, PickWant, ViewMirror, pick_kinds,
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
    let p = gz_ui::Palette::new(gz_ui::Accent::default());
    let scene = Scene::build(&SceneInput {
        graph: &g,
        catalog: &cat,
        edit: 1,
        palette: &p,
        wires: WireSettings::default(),
        names: None,
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
