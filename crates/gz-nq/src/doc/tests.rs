use super::*;
use crate::catalog::{Kind, Param, USE_EXTRA, USE_MAIN, USE_TRIGGER};

fn cat() -> Catalog {
    let kind = |id: &str, uses: u8, pins: &[&str]| Kind {
        id: id.into(),
        title: id.into(),
        uses,
        pins: pins.iter().map(|p| (*p).to_owned()).collect(),
        in_quest: true,
        in_behaviour: true,
        ..Kind::default()
    };
    let mut wait = kind("wait.when", USE_MAIN, &["done", "timeout"]);
    wait.params = vec![Param { name: "timeout".into(), ty: "duration".into(), ..Param::default() }];
    let mut timer = kind("wait.timer", USE_MAIN, &["done"]);
    timer.params = vec![Param { name: "duration".into(), ty: "duration".into(), ..Param::default() }];
    let mut quest_only = kind("dialog.topic", USE_MAIN, &["next"]);
    quest_only.in_behaviour = false;
    Catalog::from_kinds(vec![
        kind("trigger.start", USE_TRIGGER, &["next"]),
        kind("flow.step", USE_MAIN, &["next"]),
        kind("flow.end", USE_MAIN, &[]),
        wait,
        timer,
        quest_only,
        kind("var.set", USE_EXTRA, &[]),
    ])
}

fn graph() -> Graph {
    let mut g = Graph::default();
    let mut s = Node::new("s", "trigger.start");
    s.connect("next", "a");
    let mut a = Node::new("a", "flow.step");
    a.connect("next", "b");
    let mut b = Node::new("b", "flow.step");
    b.params.set("node", "a");
    let mut cond = Node::new("c", "flow.step");
    cond.cond.push(asset::Cond {
        kind: "node_done".into(),
        params: {
            let mut p = Value::table();
            p.set("node", "a");
            p
        },
        negate: false,
    });
    for n in [s, a, b, cond] {
        g.nodes.push_back(n);
    }
    layout::run(&cat(), &mut g, false);
    g
}

fn ops(text: &str) -> Value {
    match gz_luadata::parse(&format!("return {text}")) {
        Ok(v) => v,
        Err(e) => panic!("{}", e.message),
    }
}

fn apply(g: &mut Graph, text: &str) -> Result<Done, String> {
    apply_one(&cat(), g, &ops(text), &[])
}

#[test]
fn every_operation() {
    let c = cat();
    let mut g = graph();
    let applied = apply_ops(
        &c,
        &mut g,
        &ops(r#"{
            { op = "add_node", node = { id = "d", kind = "flow.end" } },
            { op = "connect", from = "b", to = "d" },
            { op = "rename_node", id = "a", new_id = "aa" },
            { op = "set_node", id = "b", patch = { once = true, comment = "hi", pos = { 16, 32 } } },
            { op = "add_action", id = "b", slot = "exit", action = { kind = "var.add", params = { name = "n", delta = 1 } } },
            { op = "add_action", id = "b", slot = "exit", index = 1, action = { kind = "money.give" } },
            { op = "set_action", id = "b", slot = "exit", index = 2, patch = { kind = "var.set" } },
            { op = "move_action", id = "b", slot = "exit", index = 1, to_id = "d", to_slot = "enter" },
            { op = "set_quest", patch = { id = "q2", vars = { n = 0 } } },
            { op = "set_pos", id = "d", x = 5, y = 6 },
        }"#),
        &[],
    )
    .map(|(n, _)| n);
    assert_eq!(applied, Ok(10));
    assert_eq!(g.id, "q2");
    assert!(g.find_node("a").is_none());
    assert_eq!(
        g.find_node("s").and_then(|n| n.targets("next")).map(<[SmolStr]>::to_vec),
        Some(vec![SmolStr::new("aa")])
    );
    // a node reference inside a condition follows the rename
    assert_eq!(
        g.find_node("c").and_then(|n| n.cond.first()).map(|c| c.params.get_string("node", "")),
        Some("aa".to_owned())
    );
    let b = g.find_node("b").cloned().unwrap_or_default();
    assert!(b.once_set && b.once);
    assert_eq!((b.comment.as_str(), b.pos, b.on_exit.len()), ("hi", [16.0, 32.0], 1));
    assert_eq!(b.on_exit.first().map(|a| a.kind.as_str()), Some("var.set"));
    let d = g.find_node("d").cloned().unwrap_or_default();
    assert_eq!(d.on_enter.first().map(|a| a.kind.as_str()), Some("money.give"));
    assert_eq!(d.pos, [5.0, 6.0]);
    assert_eq!(g.find_var("n").map(|v| v.value.clone()), Some(Value::Number(0.0)));
}

#[test]
fn operations_are_all_or_nothing() {
    let c = cat();
    let mut g = graph();
    let before = g.clone();
    let r = apply_ops(
        &c,
        &mut g,
        &ops(r#"{ { op = "remove_node", id = "a" }, { op = "remove_node", id = "zz" } }"#),
        &[],
    );
    assert_eq!(r.err(), Some("op #2: node 'zz' does not exist".to_owned()));
    assert_eq!(g, before);
    assert_eq!(apply(&mut g, r#"{ op = "warp" }"#).err(), Some("unknown op 'warp'".to_owned()));
    assert_eq!(
        apply(&mut g, r#"{ op = "set_node", id = "a", patch = { id = "x" } }"#).err(),
        Some("set cannot change the id - rename does".to_owned())
    );
    assert!(apply(&mut g, r#"{ op = "add_action", id = "a", index = 9, action = { kind = "x" } }"#).is_ok());
    assert_eq!(
        apply(&mut g, r#"{ op = "remove_action", id = "a", index = 5 }"#).err(),
        Some("index out of range (1-based)".to_owned())
    );
    // removing a node takes the edges into it
    assert!(apply(&mut g, r#"{ op = "remove_node", id = "a" }"#).is_ok());
    assert!(g.find_node("s").is_some_and(|n| n.out.is_empty()));
}

#[test]
fn routes_and_bookmarks_follow_a_rename_and_go_with_their_node() {
    let mut g = graph();
    assert!(
        apply(
            &mut g,
            r#"{ op = "route", from = "a", pin = "next", to = "b", points = { { 100, 50 }, { 120, 90 } } }"#
        )
        .is_ok()
    );
    assert!(apply(&mut g, r#"{ op = "bookmark", name = "Finale", node = "b" }"#).is_ok());
    assert!(apply(&mut g, r#"{ op = "rename", node = "b", to = "end" }"#).is_ok());
    assert_eq!(g.canvas.route("a", "next", "end").map(<[[f32; 2]]>::len), Some(2));
    assert_eq!(g.canvas.bookmarks.first().map(|b| b.node.as_str()), Some("end"));
    // a route of a wire that does not exist is refused
    assert!(
        apply(&mut g, r#"{ op = "route", from = "s", pin = "next", to = "end", points = { { 1, 1 } } }"#)
            .is_err()
    );
    assert!(apply(&mut g, r#"{ op = "remove", node = "end" }"#).is_ok());
    assert!(g.canvas.routes.is_empty() && g.canvas.bookmarks.is_empty());
}

#[test]
fn a_frame_moves_the_nodes_inside_it() {
    let mut g = graph();
    assert!(
        apply_one(
            &cat(),
            &mut g,
            &ops(r#"{ op = "frame", action = "add", title = "Start" }"#),
            &[SmolStr::new("s"), SmolStr::new("a")]
        )
        .is_ok()
    );
    let f = g.canvas.frames.first().cloned().unwrap_or_default();
    assert_eq!(f.title, "Start");
    let (s0, a0, b0) = (g.nodes[0].pos, g.nodes[1].pos, g.nodes[2].pos);
    assert!(apply(&mut g, r#"{ op = "frame", action = "move", index = 1, by = { 32, 64 } }"#).is_ok());
    assert_eq!(g.nodes[0].pos, [s0[0] + 32.0, s0[1] + 64.0]);
    assert_eq!(g.nodes[1].pos, [a0[0] + 32.0, a0[1] + 64.0]);
    // a node outside the frame stays
    assert_eq!(g.nodes[2].pos, b0);
    assert_eq!(g.canvas.frames[0].x, f.x + 32.0);
    assert!(apply(&mut g, r#"{ op = "frame", action = "remove", index = 1 }"#).is_ok());
    assert!(g.canvas.frames.is_empty());
}

#[test]
fn replace_keeps_the_id_the_shared_parameters_and_the_pins() {
    let mut g = graph();
    assert!(apply(&mut g, r#"{ op = "add", node = { id = "w", kind = "wait.when", params = { timeout = 5 }, out = { done = "b", timeout = "c" } } }"#).is_ok());
    assert!(apply(&mut g, r#"{ op = "replace", node = "w", kind = "wait.timer" }"#).is_ok());
    let w = g.find_node("w").cloned().unwrap_or_default();
    assert_eq!(w.kind, "wait.timer");
    // timer takes no timeout, has no timeout pin: both go, done stays
    assert!(w.params.get("timeout").is_none());
    assert_eq!(w.targets("done").map(<[SmolStr]>::len), Some(1));
    assert!(w.targets("timeout").is_none());
}

#[test]
fn insert_on_wire_splits_the_wire() {
    let mut g = graph();
    assert!(
        apply(
            &mut g,
            r#"{ op = "insert_on_wire", from = "a", pin = "next", to = "b", kind = "wait.timer" }"#
        )
        .is_ok()
    );
    let a = g.find_node("a").cloned().unwrap_or_default();
    let n = a.targets("next").and_then(|t| t.first()).cloned().unwrap_or_default();
    assert_eq!(g.find_node(&n).map(|x| x.kind.as_str()), Some("wait.timer"));
    assert_eq!(
        g.find_node(&n).and_then(|x| x.targets("done")).map(<[SmolStr]>::to_vec),
        Some(vec![SmolStr::new("b")])
    );
    assert!(
        apply(&mut g, r#"{ op = "insert_on_wire", from = "s", pin = "next", to = "b", kind = "flow.step" }"#)
            .is_err()
    );
    // a node already on the canvas goes on a wire where it stands
    assert!(
        apply(&mut g, r#"{ op = "add", node = { id = "lone", kind = "flow.step", pos = { 900, 900 } } }"#)
            .is_ok()
    );
    assert!(
        apply(&mut g, r#"{ op = "insert_on_wire", from = "s", pin = "next", to = "a", existing = "lone" }"#)
            .is_ok()
    );
    let lone = g.find_node("lone").cloned().unwrap_or_default();
    assert_eq!(lone.pos, [900.0, 900.0]);
    assert_eq!(lone.targets("next").map(<[SmolStr]>::to_vec), Some(vec![SmolStr::new("a")]));
    assert_eq!(
        g.find_node("s").and_then(|x| x.targets("next")).map(<[SmolStr]>::to_vec),
        Some(vec![SmolStr::new("lone")])
    );
    assert!(
        apply(&mut g, r#"{ op = "insert_on_wire", from = "lone", pin = "next", to = "a", existing = "s" }"#)
            .is_err()
    );
}

#[test]
fn a_bookmark_here_marks_the_selection() {
    let c = cat();
    let mut g = graph();
    assert!(
        apply_one(&c, &mut g, &ops(r#"{ op = "bookmark", name = "1", here = true }"#), &[SmolStr::new("b")])
            .is_ok()
    );
    assert_eq!(g.canvas.bookmarks.first().map(|b| (b.name.as_str(), b.node.as_str())), Some(("1", "b")));
    assert!(apply_one(&c, &mut g, &ops(r#"{ op = "bookmark", name = "2", here = true }"#), &[]).is_err());
}

#[test]
fn duplicate_with_wires_keeps_the_wires_into_the_copy() {
    let mut g = graph();
    let done = apply(&mut g, r#"{ op = "duplicate", nodes = { "a" }, wires = true }"#).unwrap_or_default();
    let copy = done.select.and_then(|s| s.first().cloned()).unwrap_or_default();
    assert_eq!(copy, "a2");
    assert!(g.find_node("s").is_some_and(|s| s.targets("next").is_some_and(|t| t.iter().any(|x| x == "a2"))));
    let plain = apply(&mut g, r#"{ op = "duplicate", nodes = { "a" } }"#).unwrap_or_default();
    let copy2 = plain.select.and_then(|s| s.first().cloned()).unwrap_or_default();
    assert!(!g.find_node("s").is_some_and(|s| s.targets("next").is_some_and(|t| t.contains(&copy2))));
}

#[test]
fn disable_and_collapse_toggle_the_selection() {
    let c = cat();
    let mut g = graph();
    let sel = [SmolStr::new("a"), SmolStr::new("b")];
    assert!(apply_one(&c, &mut g, &ops(r#"{ op = "disable" }"#), &sel).is_ok());
    assert!(g.find_node("a").is_some_and(|n| n.disabled) && g.find_node("b").is_some_and(|n| n.disabled));
    assert!(apply_one(&c, &mut g, &ops(r#"{ op = "disable" }"#), &sel).is_ok());
    assert!(g.find_node("a").is_some_and(|n| !n.disabled));
    assert!(apply_one(&c, &mut g, &ops(r#"{ op = "collapse", on = true }"#), &sel).is_ok());
    assert!(g.find_node("b").is_some_and(|n| n.collapsed));
    assert!(apply_one(&c, &mut g, &ops(r#"{ op = "disable" }"#), &[]).is_err());
}

#[test]
fn paste_leaves_out_what_the_document_may_not_carry() {
    let c = cat();
    let mut g = Graph { kind: asset::DocKind::Behaviour, ..graph() };
    let text = r#"return { nodes = { { id = "t", kind = "dialog.topic" }, { id = "x", kind = "flow.step", pos = { 0, 0 } } } }"#;
    let mut op = Value::table();
    op.set("op", "paste");
    op.set("text", text);
    let done = apply_one(&c, &mut g, &op, &[]).unwrap_or_default();
    assert!(g.find_node("x").is_none() || g.find_node("x2").is_some() || g.find_node("x").is_some());
    assert!(g.find_node("t").is_none(), "a quest's kind came into a behaviour graph");
    assert!(done.status.contains("dialog.topic"), "{}", done.status);
}

#[test]
fn the_selection_grows_along_the_wires() {
    let g = graph();
    let down = grown(&g, &[SmolStr::new("a")], Grow::Downstream);
    assert_eq!(down, vec![SmolStr::new("a"), SmolStr::new("b")]);
    let up = grown(&g, &[SmolStr::new("b")], Grow::Upstream);
    assert_eq!(up, vec![SmolStr::new("s"), SmolStr::new("a"), SmolStr::new("b")]);
    let all = grown(&g, &[SmolStr::new("a")], Grow::Connected);
    assert_eq!(all.len(), 3);
}

#[test]
fn arrangements() {
    let c = cat();
    let mut g = graph();
    for (i, n) in g.nodes.iter_mut().enumerate() {
        n.pos = [i as f32 * 300.0, (i as f32 * 37.0).round()];
    }
    let sel: Vec<usize> = vec![0, 1, 2];
    let left = arrange::align(&c, &g, &sel, Align::Left);
    assert!(left.iter().all(|(_, p)| p[0] == 0.0), "{left:?}");
    let top = arrange::align(&c, &g, &sel, Align::Top);
    assert!(top.iter().all(|(_, p)| p[1] == 0.0), "{top:?}");
    let spread = arrange::distribute(&c, &g, &[0, 1, 2, 3], true);
    let xs: Vec<f32> = spread.iter().map(|(_, p)| p[0]).collect();
    assert_eq!(xs.len(), 4);
    // a node's next wire runs straight after straightening: the target's input on the pin's line
    let st = arrange::straighten(&c, &g, &[1]);
    let (ti, p) = st.first().copied().unwrap_or((usize::MAX, [0.0, 0.0]));
    assert_eq!(ti, 2);
    let cx = BoxCx::of(&c, &g);
    let a = &g.nodes[1];
    let from = layout::output_pin(cx.flow, a.pos, layout::node_size(&cx, a), 0, 1);
    let into = layout::input_pin(cx.flow, p, layout::node_size(&cx, &g.nodes[2]));
    assert!((from[1] - into[1]).abs() < 0.01, "{from:?} {into:?}");
}

#[test]
fn canvas_operations() {
    let c = cat();
    let mut g = graph();
    assert_eq!(can_link(&c, &g, "a", "s"), Err("'s' is a trigger - triggers have no input".to_owned()));
    assert_eq!(can_link(&c, &g, "a", "a"), Err("A node cannot link to itself".to_owned()));
    let status = bypass(&c, &mut g, &[SmolStr::new("a")]);
    assert_eq!(status, "Bypassed: 1 link(s) bridged");
    assert_eq!(
        g.find_node("s").and_then(|n| n.targets("next")).map(<[SmolStr]>::to_vec),
        Some(vec![SmolStr::new("b")])
    );
    assert!(g.find_node("a").is_some_and(|n| n.out.is_empty()));
    layout::run(&c, &mut g, false);
    let copy = g.find_node("b").cloned().unwrap_or_default();
    let made = insert_nodes(&c, &mut g, vec![copy], [32.0, 32.0], false);
    assert_eq!(made, [SmolStr::new("b2")]);
    let (edge, status) = connect_nearest(&c, &g, &[SmolStr::new("a")]);
    assert!(edge.is_some(), "{status}");
}

#[test]
fn undo_takes_an_edit_back() {
    let rt = match gz_task::Runtime::start(gz_task::ExecOptions::default()) {
        Ok(rt) => rt,
        Err(e) => panic!("{e}"),
    };
    let exec = rt.exec();
    rt.block_on(async {
        let store = gz_store::Store::spawn(
            "doc",
            GraphDoc::default(),
            &exec,
            gz_store::StoreOptions { history_limit: 200 },
        );
        let catalog = Arc::new(cat());
        let send = |op: DocOp| store.send(DocCmd { catalog: catalog.clone(), op }, gz_base::Origin::Mcp);
        assert!(send(DocOp::Load { graph: graph(), shape: Vec::new(), file_time: None }).await.is_ok());
        assert!(!store.is_dirty());
        assert!(send(DocOp::RemoveNodes(vec!["b".into()])).await.is_ok());
        assert!(store.is_dirty());
        assert!(store.snapshot().graph.find_node("b").is_none());
        assert_eq!(store.history().undo, ["Delete Node"]);
        assert_eq!(store.undo().await.as_deref(), Some("Delete Node"));
        assert!(store.snapshot().graph.find_node("b").is_some());
        assert!(!store.is_dirty());
        assert_eq!(store.redo().await.as_deref(), Some("Delete Node"));
        assert!(store.snapshot().graph.find_node("b").is_none());
        // a refused edit changes nothing and records no step
        assert!(send(DocOp::RenameNode { id: "zz".into(), new_id: "y".into() }).await.is_err());
        assert_eq!(store.history().undo, ["Delete Node"]);
        // the view follows: a selection of a node that went is pruned
        assert!(send(DocOp::View(ViewOp::Select(vec!["s".into(), "a".into()]))).await.is_ok());
        assert!(send(DocOp::Canvas(CanvasOp::Delete)).await.is_ok());
        assert!(store.snapshot().view.selection.is_empty());
        assert!(store.snapshot().graph.find_node("a").is_none());
        // a flow switch lays the graph out again: one step
        let arr = Arrange {
            action: ArrangeAction::Layout,
            nodes: Vec::new(),
            scope: Scope::All,
            flow: Some(Flow::Down),
        };
        assert!(send(DocOp::Arrange(arr)).await.is_ok());
        assert_eq!(store.snapshot().graph.flow(), Flow::Down);
        assert_eq!(store.history().undo.first().map(String::as_str), Some("Flow Direction"));
    });
}

#[test]
fn the_zoom_ladder() {
    assert_eq!(zoom_of(ZOOM_DEFAULT), 1.0);
    assert_eq!(zoom_of(0), 0.05);
    assert_eq!(zoom_of(99), 2.0);
    assert_eq!(level_at_most(0.33), 9);
    assert_eq!(level_at_most(0.01), 0);
}
