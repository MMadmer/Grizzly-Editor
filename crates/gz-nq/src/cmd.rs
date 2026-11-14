//! The `graph.*` commands of open documents: `graph.edit` (every edit the canvas can make, one
//! undo step), `graph.arrange` (layout, align, distribute, straighten), `graph.get` (the graph as
//! the tab holds it) and `graph.find`. A graph's file is the author's: it is created, read and
//! changed with file tools, checked with `asset.check` and searched with `asset.find`; these
//! commands work on what the editor has open. `graph.view` is the tab's own (`gz-ui-graph`).

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, CmdError, Editor, Registry, Reply, Spec, Topic};
use gz_project::{AssetTemplates, ProjectHooks, TemplateKind};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::arrange::{Align, Scope};
use crate::asset::{self, Graph};
use crate::canvas::Flow;
use crate::check;
use crate::defaults::GraphDefaults;
use crate::doc::{self, Arrange, ArrangeAction, DocOp, ViewOp};
use crate::layout::{self, BoxCx};
use crate::pickers::{PickerEntry, PickerType};
use crate::service::{DocHandle, NqDirty, NqService};
use crate::text::{self, contains_no_case};
use crate::validate::{self, NqProblem};
use crate::wires;

fn refused(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

fn service(ed: &Editor) -> Result<Arc<NqService>, CmdError> {
    NqService::of(ed)
}

fn r1(v: f32) -> Value {
    json!((f64::from(v) * 10.0).round() / 10.0)
}

/// The file of a graph a `doc` names: its tab's id (`quest:<path>`, `beh:<path>`) or the path.
#[must_use]
pub fn doc_path(doc: &str) -> &str {
    doc.strip_prefix("quest:").or_else(|| doc.strip_prefix("beh:")).unwrap_or(doc)
}

/// The tab id of a graph's file.
#[must_use]
pub fn doc_id(wire: &str) -> String {
    let wire = wire.replace('\\', "/");
    if asset::is_behaviour_path(&wire) { format!("beh:{wire}") } else { format!("quest:{wire}") }
}

/// Opens (or finds) the document a `doc` names.
async fn open_doc(ed: &Arc<Editor>, svc: &NqService, doc: &str) -> Result<Arc<DocHandle>, CmdError> {
    let path = doc_path(doc);
    if path.is_empty() {
        return Err(CmdError::invalid("doc names the graph: quest:<path>, beh:<path> or its file"));
    }
    svc.open(ed, path)
        .await
        .map_err(|e| if e.starts_with("file does not exist") { CmdError::not_found(e) } else { refused(e) })
}

/// The graph's tab brought up, when there is a window: an edit is seen where it is made, and the
/// tab's Save and Undo reach it.
async fn bring_up(ed: &Arc<Editor>, d: &DocHandle) {
    if !ed.frames.has_window() {
        return;
    }
    let wire = d.path().wire;
    let id = doc_id(&wire);
    let mut a = Args::new();
    a.insert("action".into(), json!("open"));
    a.insert("doc".into(), json!(id));
    a.insert("kind".into(), json!(id.split(':').next().unwrap_or("quest")));
    a.insert("path".into(), json!(wire));
    let reply = ed.call("ui.tab", a, ed.ui_origin()).await;
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        ed.log.detail(format!(
            "[nq] the tab of {wire} did not come up: {}",
            reply.get("error").and_then(Value::as_str).unwrap_or("")
        ));
    }
}

/// Saves when asked, and always in an editor without a window: there no tab holds the edit.
async fn settle(ed: &Arc<Editor>, svc: &NqService, d: &DocHandle, save: bool) -> Result<bool, CmdError> {
    if !(save || !ed.frames.has_window()) || !d.is_dirty() {
        return Ok(false);
    }
    svc.save(ed, d).await.map_err(|e| CmdError::new("io", e))?;
    Ok(true)
}

/// The problems of a document: their sentence, their code, the node and field, their place in
/// the file.
fn problems_json(d: &DocHandle, problems: &[NqProblem]) -> Value {
    let placed = d.positioned(problems);
    Value::Array(
        problems
            .iter()
            .zip(placed)
            .map(|(p, q)| {
                let mut o = serde_json::to_value(&q).unwrap_or_else(|_| json!({}));
                if let Some(m) = o.as_object_mut() {
                    m.insert("node".into(), json!(p.node));
                    m.insert("slot".into(), json!(p.slot));
                }
                o
            })
            .collect(),
    )
}

/// `doc`, `path`, `dirty`, `undo`, `redo` and what the file on disk says the document does not
/// show: `disk_error`, `missing`, `conflicts`.
fn doc_state(out: &mut Map<String, Value>, d: &DocHandle) {
    let p = d.path();
    out.insert("doc".into(), json!(doc_id(&p.wire)));
    out.insert("path".into(), json!(p.wire));
    out.insert("dirty".into(), json!(d.is_dirty()));
    let h = d.store.history();
    out.insert("undo".into(), json!(h.undo.len()));
    out.insert("redo".into(), json!(h.redo.len()));
    let disk = d.disk();
    if let Some(e) = disk.error {
        out.insert("disk_error".into(), json!(e));
    }
    if disk.missing {
        out.insert("missing".into(), json!(true));
    }
    if !disk.conflicts.is_empty() {
        let list: Vec<Value> = disk
            .conflicts
            .iter()
            .map(|c| {
                json!({
                    "path": gz_luadata::path::show(&c.path),
                    "mine": c.mine.as_ref().map(gz_luadata::to_json),
                    "theirs": c.theirs.as_ref().map(gz_luadata::to_json),
                    "line": c.span.map(|s| s.line),
                })
            })
            .collect();
        out.insert("conflicts".into(), Value::Array(list));
    }
}

/// What an edit or an arrangement answers: the document's state, the selection, the status, the
/// outline and the counts of its problems.
async fn after_edit(ed: &Arc<Editor>, svc: &NqService, d: &DocHandle, saved: bool) -> Map<String, Value> {
    let catalog = svc.catalog(ed).await;
    let problems = svc.problems(ed, d).await;
    let snap = d.store.snapshot();
    let mut out = Map::new();
    doc_state(&mut out, d);
    out.insert("saved".into(), json!(saved));
    out.insert("selected".into(), json!(snap.view.selection));
    out.insert("status".into(), json!(snap.view.status));
    out.insert("errors".into(), json!(validate::count_errors(&problems)));
    out.insert("warnings".into(), json!(validate::count_warnings(&problems)));
    out.insert("outline".into(), json!(asset::outline(&catalog, &snap.graph)));
    out
}

// ---- graph.edit -------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditParams {
    /// The graph: its tab (`quest:<path>`, `beh:<path>`) or its file, project-relative.
    doc: String,
    /// The operations, in order, all or nothing, one undo step: a list of `{"op": ..., ...}`
    /// (`help about=graph-ops` lists them), one such object, or the text of a Lua list.
    ops: Value,
    /// Writes the file right after. An editor without a window always does: no tab holds the
    /// edit there.
    #[serde(default)]
    save: bool,
}

fn ops_value(ops: &Value) -> Result<gz_luadata::Value, CmdError> {
    match ops {
        Value::String(s) => {
            let t = text::trim(s);
            let chunk = if t.starts_with('{') { format!("return {t}") } else { t.to_owned() };
            asset::parse_value(&chunk, "ops")
                .map_err(|e| CmdError::invalid(format!("ops: {}", e.with_line())))
        }
        Value::Array(_) | Value::Object(_) => Ok(gz_luadata::from_json(ops)),
        _ => Err(CmdError::invalid("ops is a list of operations: [{\"op\": \"add\", ...}, ...]")),
    }
}

async fn graph_edit(ed: Arc<Editor>, p: EditParams, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = open_doc(&ed, &svc, &p.doc).await?;
    let ops = ops_value(&p.ops)?;
    let list: Vec<gz_luadata::Value> =
        if ops.arr().is_empty() && ops.has("op") { vec![ops.clone()] } else { ops.arr().to_vec() };
    // a task's id is kept in saves: its rename is a step of its own, acknowledged
    if let Some(rt) = list.iter().find(|op| op.get_string("op", "") == "rename_task") {
        if list.len() > 1 {
            return Err(CmdError::invalid("rename_task runs alone: one operation in its graph.edit"));
        }
        if !rt.get_bool("ack_runtime_identity", false) {
            return Err(refused(
                "a task's id is kept in the game's saves: a rename leaves the old state behind - pass ack_runtime_identity=true",
            ));
        }
        let wire = d.path().wire;
        let reply = svc
            .send(
                &ed,
                &d,
                DocOp::RenameTask {
                    from: rt.get_string("from", ""),
                    to: rt.get_string("to", ""),
                    path: wire,
                },
                o,
            )
            .await?;
        bring_up(&ed, &d).await;
        let saved = settle(&ed, &svc, &d, p.save).await?;
        let mut out = after_edit(&ed, &svc, &d, saved).await;
        out.insert("references_updated".into(), json!(reply.count));
        return Ok(Value::Object(out));
    }
    let reply = svc.send(&ed, &d, DocOp::ApplyOps(ops), o).await?;
    bring_up(&ed, &d).await;
    let saved = settle(&ed, &svc, &d, p.save).await?;
    let mut out = after_edit(&ed, &svc, &d, saved).await;
    out.insert("applied".into(), json!(reply.count));
    Ok(Value::Object(out))
}

// ---- graph.arrange ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ArrangeWhat {
    Layout,
    Align,
    Distribute,
    Straighten,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ScopeWhat {
    All,
    Selection,
    Branch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FlowWhat {
    Right,
    Down,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ArrangeParams {
    /// The graph: its tab (`quest:<path>`, `beh:<path>`) or its file, project-relative.
    doc: String,
    /// layout (the auto layout), align, distribute or straighten (the successors of the nodes
    /// move so their wires run straight).
    action: ArrangeWhat,
    /// For align: left, centre, right, top, middle or bottom. For distribute: horizontal or
    /// vertical.
    #[serde(default)]
    how: Option<String>,
    /// The nodes, by id. Left out: the selection.
    #[serde(default)]
    nodes: Option<Vec<String>>,
    /// For layout: all (default), selection (the nodes alone, where they were) or branch (the one
    /// node given and what only it leads to, from where it stands).
    #[serde(default)]
    scope: Option<ScopeWhat>,
    /// For layout: the graph flows this way from now on (right or down), laid out again.
    #[serde(default)]
    flow: Option<FlowWhat>,
    /// Writes the file right after (always without a window).
    #[serde(default)]
    save: bool,
}

async fn graph_arrange(ed: Arc<Editor>, p: ArrangeParams, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = open_doc(&ed, &svc, &p.doc).await?;
    let how = p.how.clone().unwrap_or_default();
    let action = match p.action {
        ArrangeWhat::Layout => ArrangeAction::Layout,
        ArrangeWhat::Align => ArrangeAction::Align(Align::parse(&how).ok_or_else(|| {
            CmdError::invalid("how says where to align: left, centre, right, top, middle or bottom")
        })?),
        ArrangeWhat::Distribute => ArrangeAction::Distribute {
            horizontal: match how.as_str() {
                "" | "horizontal" | "horizontally" => true,
                "vertical" | "vertically" => false,
                _ => return Err(CmdError::invalid("how is horizontal or vertical")),
            },
        },
        ArrangeWhat::Straighten => ArrangeAction::Straighten,
    };
    let scope = match p.scope.unwrap_or(ScopeWhat::All) {
        ScopeWhat::All => Scope::All,
        ScopeWhat::Selection => Scope::Selection,
        ScopeWhat::Branch => Scope::Branch,
    };
    let flow = p.flow.map(|f| if f == FlowWhat::Down { Flow::Down } else { Flow::Right });
    let arr = Arrange { action, nodes: p.nodes.unwrap_or_default(), scope, flow };
    let reply = svc.send(&ed, &d, DocOp::Arrange(arr), o).await?;
    bring_up(&ed, &d).await;
    let saved = settle(&ed, &svc, &d, p.save).await?;
    let catalog = svc.catalog(&ed).await;
    let snap = d.store.snapshot();
    let mut out = Map::new();
    doc_state(&mut out, &d);
    out.insert("saved".into(), json!(saved));
    out.insert("moved".into(), json!(reply.count));
    out.insert("flow".into(), json!(snap.graph.flow().key()));
    out.insert("boxes".into(), boxes_json(&catalog, &snap.graph));
    Ok(Value::Object(out))
}

/// Every node's box as the canvas draws it: content-sized, the same one the layout made room for.
fn boxes_json(catalog: &crate::Catalog, g: &Graph) -> Value {
    let cx = BoxCx::of(catalog, g);
    Value::Array(
        g.nodes
            .iter()
            .map(|n| {
                let s = layout::node_size(&cx, n);
                json!({
                    "id": n.id, "kind": n.kind, "x": r1(n.pos[0]), "y": r1(n.pos[1]), "w": r1(s[0]), "h": r1(s[1]),
                    "collapsed": n.collapsed, "disabled": n.disabled,
                })
            })
            .collect(),
    )
}

// ---- graph.get --------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetParams {
    /// The graph: its tab (`quest:<path>`, `beh:<path>`) or its file, project-relative.
    doc: String,
    /// Every node's box (x, y, w, h in world pt).
    #[serde(default)]
    boxes: bool,
    /// Every wire's polyline in the style it is drawn in, and whether it goes back.
    #[serde(default)]
    wires: bool,
    /// The view the tab holds: zoom, centre, selection, the history, the minimap.
    #[serde(default)]
    view: bool,
    /// What is under a world point `[x, y]`: the node whose box holds it, else the wire nearest to
    /// it within 10 pt - the canvas's hit test, on the same boxes and polylines.
    #[serde(default)]
    at: Option<[f32; 2]>,
}

/// The node, else the wire, under a world point.
fn hit_at(
    catalog: &crate::Catalog,
    g: &Graph,
    defaults: &crate::canvas::WireSettings,
    at: [f32; 2],
) -> Value {
    let cx = BoxCx::of(catalog, g);
    // the last drawn is on top
    let node = g.nodes.iter().rev().find(|n| {
        let s = layout::node_size(&cx, n);
        at[0] >= n.pos[0] && at[0] <= n.pos[0] + s[0] && at[1] >= n.pos[1] && at[1] <= n.pos[1] + s[1]
    });
    if let Some(n) = node {
        return json!({ "node": n.id, "wire": Value::Null });
    }
    let mut best = 10.0f32;
    let mut wire = Value::Null;
    for (from, pin, to, w) in wires_of(catalog, g, defaults) {
        let d = wires::distance(&w.points, at);
        if d <= best {
            best = d;
            wire = json!({ "from": from, "pin": pin, "to": to, "distance": r1(d) });
        }
    }
    json!({ "node": Value::Null, "wire": wire })
}

/// Every wire of a graph as the canvas draws it: its pins, its style, its waypoints.
#[must_use]
pub fn wires_of(
    catalog: &crate::Catalog,
    g: &Graph,
    defaults: &crate::canvas::WireSettings,
) -> Vec<(String, String, String, wires::Wire)> {
    let cx = BoxCx::of(catalog, g);
    let set = g.canvas.wires.over(defaults);
    let wcx = wires::WireCx { flow: cx.flow, style: set.style, diagonal: set.diagonal };
    let boxes: Vec<[f32; 4]> = g
        .nodes
        .iter()
        .map(|n| {
            let s = layout::node_size(&cx, n);
            [n.pos[0], n.pos[1], n.pos[0] + s[0], n.pos[1] + s[1]]
        })
        .collect();
    let index = g.index();
    let mut lanes = Vec::new();
    let mut out = Vec::new();
    for n in &g.nodes {
        let size = layout::node_size(&cx, n);
        let (pins, _) = layout::pins(catalog, n);
        let count: usize = n.out.iter().map(|p| p.targets.len()).sum();
        let mut k = 0;
        for (pi, pin) in pins.iter().enumerate() {
            let Some(p) = n.out.iter().find(|p| p.name.as_str() == pin) else { continue };
            for (ti, t) in p.targets.iter().enumerate() {
                let Some(m) = index.get(t.as_str()).and_then(|&j| g.nodes.get(j)) else { continue };
                let p0 = layout::output_pin(cx.flow, n.pos, size, pi, pins.len());
                let p3 = layout::input_pin(cx.flow, m.pos, layout::node_size(&cx, m));
                let via = g.canvas.route(&n.id, pin, t).unwrap_or(&[]);
                let lanes_of = wires::Lanes { index: k, count, track: ti, tracks: p.targets.len() };
                let w = wires::build(&wcx, p0, p3, via, lanes_of, &boxes, &mut lanes);
                out.push((n.id.to_string(), pin.clone(), t.to_string(), w));
                k += 1;
            }
        }
    }
    out
}

async fn graph_get(ed: Arc<Editor>, p: GetParams, _o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = open_doc(&ed, &svc, &p.doc).await?;
    let catalog = svc.catalog(&ed).await;
    let problems = svc.problems(&ed, &d).await;
    let snap = d.store.snapshot();
    let g = &snap.graph;
    let defaults = GraphDefaults::of(&ed).wires();
    let set = g.canvas.wires.over(&defaults);
    let mut out = Map::new();
    doc_state(&mut out, &d);
    out.insert("kind".into(), json!(if g.is_behaviour() { "behaviour" } else { "quest" }));
    out.insert("id".into(), json!(g.id));
    out.insert("title".into(), json!(text::preview(&g.title)));
    out.insert("nodes".into(), json!(g.nodes.len()));
    out.insert("flow".into(), json!(g.flow().key()));
    out.insert(
        "wire_style".into(),
        json!({
            "style": set.style.key(), "diagonal": set.diagonal.key(), "thickness": set.thickness,
            "by_pin": set.by_pin, "dim": set.dim, "bubbles": set.bubbles, "arrows": set.arrows,
            "own": g.canvas.wires.style.is_some(),
        }),
    );
    out.insert("outline".into(), json!(asset::outline(&catalog, g)));
    out.insert("errors".into(), json!(validate::count_errors(&problems)));
    out.insert("warnings".into(), json!(validate::count_warnings(&problems)));
    out.insert("problems".into(), problems_json(&d, &problems));
    let frames: Vec<Value> = g
        .canvas
        .frames
        .iter()
        .map(|f| json!({ "title": f.title, "tone": f.tone, "x": r1(f.x), "y": r1(f.y), "w": r1(f.w), "h": r1(f.h) }))
        .collect();
    let routes: Vec<Value> = g
        .canvas
        .routes
        .iter()
        .map(|(k, pts)| json!({ "wire": k, "points": pts.iter().map(|p| json!([r1(p[0]), r1(p[1])])).collect::<Vec<_>>() }))
        .collect();
    let marks: Vec<Value> =
        g.canvas.bookmarks.iter().map(|b| json!({ "name": b.name, "node": b.node })).collect();
    out.insert("frames".into(), Value::Array(frames));
    out.insert("routes".into(), Value::Array(routes));
    out.insert("bookmarks".into(), Value::Array(marks));
    if p.boxes {
        out.insert("boxes".into(), boxes_json(&catalog, g));
    }
    if p.wires {
        let list: Vec<Value> = wires_of(&catalog, g, &defaults)
            .into_iter()
            .map(|(from, pin, to, w)| {
                json!({
                    "from": from, "pin": pin, "to": to, "back": w.back,
                    "points": w.points.iter().map(|q| json!([r1(q[0]), r1(q[1])])).collect::<Vec<_>>(),
                })
            })
            .collect();
        out.insert("wires".into(), Value::Array(list));
    }
    if let Some(at) = p.at {
        out.insert("hit".into(), hit_at(&catalog, g, &defaults, at));
    }
    if p.view {
        let v = &snap.view;
        let h = d.store.history();
        let state = |s: &doc::ViewState| json!({ "center": [r1(s.center[0]), r1(s.center[1])], "zoom": doc::zoom_of(s.zoom), "selected": s.selection, "slot": s.slot });
        out.insert(
            "view".into(),
            json!({
                "zoom": doc::zoom_of(v.zoom), "zoom_level": v.zoom, "center": [r1(v.center[0]), r1(v.center[1])],
                "selected": v.selection, "slot": v.slot, "drawer": v.drawer,
                "minimap": { "visible": v.minimap, "shown": v.minimap_shown, "available": g.nodes.len() >= doc::MINIMAP_MIN_NODES },
                "back": v.back.iter().map(state).collect::<Vec<_>>(),
                "forward": v.forward.iter().map(state).collect::<Vec<_>>(),
                "undo_steps": h.undo, "redo_steps": h.redo,
            }),
        );
    }
    Ok(Value::Object(out))
}

// ---- graph.find -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FindGo {
    First,
    Next,
    Previous,
    Clear,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindParams {
    /// The graph: its tab (`quest:<path>`, `beh:<path>`) or its file. Left out: every graph of the
    /// project.
    #[serde(default)]
    doc: Option<String>,
    /// What to find: words over ids, kinds, parameters, conditions, actions, wires and comments.
    /// Words all have to match; quotes keep a phrase; -word leaves a word out.
    #[serde(default)]
    query: Option<String>,
    /// In a graph: the tab goes to the first, next or previous result, or clears the search.
    #[serde(default)]
    go: Option<FindGo>,
    /// At most this many results (default 100).
    #[serde(default)]
    limit: Option<usize>,
}

async fn graph_find(ed: Arc<Editor>, p: FindParams, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let limit = p.limit.unwrap_or(100).clamp(1, 1000);
    let Some(doc) = p.doc.filter(|d| !d.is_empty()) else {
        let query = p.query.unwrap_or_default();
        let index = svc.index(&ed, true).await;
        let catalog = svc.catalog(&ed).await;
        let found = crate::index::find_nodes(&index, &catalog, &query);
        let results: Vec<Value> = found
            .results
            .iter()
            .take(limit)
            .map(|(path, h, source)| json!({ "doc": doc_id(path), "path": path, "node": h.node, "kind": h.kind, "title": h.title, "match": h.matched, "source": source }))
            .collect();
        return Ok(
            json!({ "query": query, "total": found.results.len(), "results": results, "complete": found.complete }),
        );
    };
    let d = open_doc(&ed, &svc, &doc).await?;
    if let Some(q) = &p.query {
        if q.len() >= 256 {
            return Err(CmdError::invalid("query is too long (at most 255 bytes)"));
        }
        svc.send(&ed, &d, DocOp::View(ViewOp::FindQuery(q.clone())), o).await?;
    }
    let catalog = svc.catalog(&ed).await;
    let results_of = |d: &DocHandle| {
        let snap = d.store.snapshot();
        let r = doc::find_results(&catalog, &snap.graph, &snap.view.find_query);
        let i = snap.find_index(&r);
        (r, i)
    };
    match p.go {
        Some(FindGo::First) => {
            svc.send(&ed, &d, DocOp::View(ViewOp::FindGo(0)), o).await?;
        }
        Some(FindGo::Next | FindGo::Previous) => {
            let (_, i) = results_of(&d);
            let to = if p.go == Some(FindGo::Previous) { i - 1 } else { i + 1 };
            svc.send(&ed, &d, DocOp::View(ViewOp::FindGo(to)), o).await?;
        }
        Some(FindGo::Clear) => {
            svc.send(&ed, &d, DocOp::View(ViewOp::FindClear), o).await?;
        }
        None => {}
    }
    let (results, index) = results_of(&d);
    let snap = d.store.snapshot();
    let mut out = Map::new();
    out.insert("doc".into(), json!(doc_id(&d.path().wire)));
    out.insert("query".into(), json!(snap.view.find_query));
    out.insert("total".into(), json!(results.len()));
    out.insert("index".into(), json!(index));
    out.insert(
        "current".into(),
        usize::try_from(index).ok().and_then(|i| results.get(i)).map_or(Value::Null, |r| json!(r.node)),
    );
    out.insert(
        "results".into(),
        Value::Array(
            results
                .iter()
                .take(limit)
                .map(|r| json!({ "node": r.node, "kind": r.kind, "title": r.title, "match": r.matched }))
                .collect(),
        ),
    );
    out.insert("selected".into(), json!(snap.view.selection));
    Ok(Value::Object(out))
}

/// The nodes of the project's graphs a text names (the tab's Find in Project): the open documents
/// with unsaved edits laid over their files.
pub async fn project_find(ed: Arc<Editor>, args: Args) -> Reply {
    let query = args.get("query").and_then(Value::as_str).unwrap_or("").to_owned();
    let svc = service(&ed)?;
    let index = svc.index(&ed, true).await;
    let catalog = svc.catalog(&ed).await;
    let found = crate::index::find_nodes(&index, &catalog, &query);
    let limit = args.get("limit").and_then(Value::as_u64).map_or(200, |l| l.clamp(1, 1000) as usize);
    let count = limit.min(found.results.len());
    let results: Vec<Value> = found
        .results
        .iter()
        .take(count)
        .map(|(path, h, source)| json!({ "path": path, "node": h.node, "kind": h.kind, "title": h.title, "match": h.matched, "source": source }))
        .collect();
    Ok(json!({
        "query": query,
        "count": found.results.len(),
        "returned": count,
        "truncated": count < found.results.len(),
        "complete": found.complete,
        "generation": found.generation,
        "fingerprint": crate::index::fingerprint_text(found.fingerprint),
        "results": results,
        "diagnostics": found.diagnostics,
    }))
}

/// Where the project's graphs name a thing of a type the catalog knows (a task, a variable, a
/// character...), with the node and slot (the tab's References).
pub async fn references(ed: Arc<Editor>, args: Args) -> Reply {
    let get = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let (ty, value) = (get("type"), get("value"));
    if ty.is_empty() || value.is_empty() {
        return Err(CmdError::invalid("type and value name what to look for"));
    }
    let svc = service(&ed)?;
    let scope = match get("path") {
        p if p.is_empty() => None,
        p => Some(svc.resolve(&p, false).map_err(refused)?.wire),
    };
    let index = svc.index(&ed, true).await;
    let catalog = svc.catalog(&ed).await;
    let r = crate::references::find(&index, &catalog, &ty, &value, scope.as_deref()).map_err(refused)?;
    let count = r.references.len().min(2000);
    Ok(json!({
        "type": ty,
        "value": value,
        "path": scope,
        "count": r.references.len(),
        "returned": count,
        "truncated": count < r.references.len(),
        "complete": r.complete,
        "generation": r.generation,
        "fingerprint": crate::index::fingerprint_text(r.fingerprint),
        "catalog_generation": r.catalog_generation,
        "references": r.references.iter().take(count).map(crate::references::Reference::to_json).collect::<Vec<_>>(),
        "diagnostics": r.diagnostics.iter().map(crate::references::Diagnostic::to_json).collect::<Vec<_>>(),
    }))
}

// ---- game.lookup ------------------------------------------------------------------------------

/// The lists of the project's documents `game.lookup` serves beside the pickers' own.
const DOCUMENT_LISTS: [&str; 10] = [
    "quest_id",
    "behaviour_id",
    "way_path",
    "global_name",
    "character_id",
    "role",
    "task_id",
    "var_name",
    "ref_name",
    "node_id",
];

/// Every list the graphs serve through `game.lookup`: the pickers' lists over the game's data
/// and the project's documents.
fn lookup_types() -> Vec<&'static str> {
    PickerType::ALL.iter().map(|t| t.name()).chain(DOCUMENT_LISTS).collect()
}

/// One list of `game.lookup`: a picker's (ranked by the picker) or one of the project's
/// documents (a document-scoped one needs `path`).
async fn lookup(ed: Arc<Editor>, q: gz_game::LookupQuery) -> Result<gz_game::LookupAnswer, CmdError> {
    let (ty, query, path, limit) = (q.ty, q.query, q.path.unwrap_or_default(), q.want.max(1));
    let svc = service(&ed)?;
    let picker = PickerType::from_name(&ty);
    let mut doc_entries: Vec<PickerEntry> = Vec::new();
    if picker.is_none() {
        let entry = |id: String, name: String, extra: String| PickerEntry { id, name, extra };
        match ty.as_str() {
            "quest_id" | "behaviour_id" => {
                // one project index, two kinds of document: each call names its own
                let want = ty == "behaviour_id";
                let index = svc.index(&ed, true).await;
                for e in index.entries.iter().filter(|e| e.behaviour() == want) {
                    let (id, title) = if e.readable {
                        (e.graph.id.clone(), text::preview(&e.graph.title))
                    } else {
                        (String::new(), String::new())
                    };
                    doc_entries.push(entry(id, title, e.rel.clone()));
                }
            }
            "way_path" => doc_entries = svc.way_paths_ready().await,
            "global_name" => {
                for (name, value) in svc.global_vars(&ed).await {
                    doc_entries.push(entry(name, value, "mod.ltx [vars]".into()));
                }
            }
            "character_id" => {
                // a template is listed and marked: it explains itself instead of being missing
                for b in svc.characters().1.iter() {
                    let extra =
                        if b.template { "template - cannot be spawned".to_owned() } else { b.file.clone() };
                    doc_entries.push(entry(b.id.clone(), b.name.clone(), extra));
                }
            }
            "role" | "task_id" | "var_name" | "ref_name" | "node_id" => {
                if path.is_empty() {
                    return Err(refused(format!("the list {ty} is a document's: give its path")));
                }
                let (g, _) = svc.read_graph(&ed, doc_path(&path)).await.map_err(refused)?;
                match ty.as_str() {
                    "role" => {
                        for r in &g.roles {
                            doc_entries.push(entry(
                                r.name.clone(),
                                text::preview(&r.title),
                                (if r.many { "group" } else { "one" }).into(),
                            ));
                        }
                    }
                    "task_id" => {
                        for t in &g.tasks {
                            doc_entries.push(entry(t.id.clone(), text::preview(&t.title), String::new()));
                        }
                    }
                    "var_name" => {
                        for v in &g.vars {
                            doc_entries.push(entry(
                                v.name.clone(),
                                text::value_summary(&v.value, 48),
                                String::new(),
                            ));
                        }
                    }
                    "node_id" => {
                        for n in &g.nodes {
                            doc_entries.push(entry(n.id.to_string(), n.kind.to_string(), String::new()));
                        }
                    }
                    _ => {
                        // the category the picker shows a human: which ref belongs in an item slot
                        let catalog = svc.catalog(&ed).await;
                        for (r, kind) in validate::declared_refs(&g, &catalog) {
                            doc_entries.push(entry(r, String::new(), kind.to_owned()));
                        }
                    }
                }
            }
            _ => return Err(refused(format!("no list {ty}"))),
        }
    }
    let pickers = svc.pickers(&ed).await;
    let (found, total, available): (Vec<&PickerEntry>, usize, bool) = match picker {
        None => {
            let matching: Vec<&PickerEntry> = doc_entries
                .iter()
                .filter(|e| {
                    query.is_empty() || contains_no_case(&e.id, &query) || contains_no_case(&e.name, &query)
                })
                .collect();
            let total = matching.len();
            (matching.into_iter().take(limit).collect(), total, true)
        }
        Some(t) => (pickers.search(t, &query, limit), pickers.entries(t).len(), pickers.available()),
    };
    let entries = found
        .into_iter()
        .map(|e| gz_game::LookupEntry { id: e.id.clone(), name: e.name.clone(), extra: e.extra.clone() })
        .collect();
    Ok(gz_game::LookupAnswer { available, total, entries })
}

const OPS_TOPIC: &str = "graph.edit takes a list of operations, all applied or none, one undo step. Nodes are named by \
id; an operation that names none acts on the selection the tab holds.

  add            node={...} (a node table) or kind=..., id=..., at=[x,y]
  remove         node= | nodes=[...]
  rename         node=, to=
  set            node=, patch={kind, once, params, cond, on_enter, on_exit, out, pos, comment, collapsed, disabled}
  connect        from=, pin=, to=
  disconnect     from=, pin= (left out: every pin), to= (left out: the whole pin)
  cut            wires=[{from, pin, to}, ...]
  move           node=, x=, y=  |  nodes=[...], by=[dx, dy]
  replace        node=, kind=: another kind, the same id; parameters of the same name and wires of the same pin stay
  insert_on_wire from=, pin=, to=, kind= (or node={...}), at=[x,y]: A -> B becomes A -> N -> B;
                 existing=<id>: a node without wires goes on the wire where it stands
  add_action     node=, slot=enter|exit, index= (1-based), action={kind, params}
  set_action     node=, slot=, index=, patch={kind, params}
  remove_action  node=, slot=, index=
  move_action    node=, slot=, index=, to_id=, to_slot=, to_index=
  set_graph      patch={id, title, activation, vars, tasks}
  set_task       task=, patch={title, descr, type, target, icon, auto_complete, objectives} | remove=true
  set_var        name=, value= (null removes) | rename=
  rename_task    from=, to=, ack_runtime_identity=true (alone in its call: the id is kept in saves)
  frame          action=add (around nodes=, or x, y, w, h), set (index=, title, tone, x, y, w, h), remove (index=),
                 move (index=, by=[dx, dy]: the nodes inside move with it)
  route          from=, pin=, to=, points=[[x, y], ...] (empty: the wire goes straight again)
  bookmark       name=, node= (here=true: the selected node; left out: the bookmark goes)
  disable        nodes=, on= (left out: toggles) - the game passes a disabled node through
  collapse       nodes=, on= - header and pins alone
  duplicate      nodes=, wires=true (the copies keep the wires into the originals)
  bypass         nodes=: A -> B -> C becomes A -> C
  connect_nearest node=
  wires          style=curved|straight|circuit|angled|subway, diagonal=start|middle|end, thickness=1-4, by_pin, dim,
                 bubbles, arrows (null: as in Preferences)
  flow           flow=right|down (graph.arrange action=layout flow= also lays the graph out again)
  paste          text= (a Lua fragment: return { nodes = { ... } }), at=[x,y]";

/// The `graph.*` commands of open documents (`graph.view` is the tab's own).
pub fn register(reg: &mut Registry) {
    reg.area("graph", "An open quest or behaviour graph: edit operations, layout, find, its view");
    reg.cmd(
        Spec::write("graph.edit", "Edits an open graph: a list of operations, all or none, one undo step.")
            .example("doc=quest:quests/wolf.nqasset ops=[{\"op\":\"connect\",\"from\":\"a\",\"pin\":\"next\",\"to\":\"b\"}]"),
        graph_edit,
    );
    reg.cmd(
        Spec::write(
            "graph.arrange",
            "Lays a graph out (all, the selection, a branch; in a flow), or aligns, distributes or straightens nodes.",
        )
        .example("doc=quest:quests/wolf.nqasset action=layout flow=right"),
        graph_arrange,
    );
    reg.cmd(
        Spec::read("graph.get", "A graph as its tab holds it: the outline, problems, frames, routes, bookmarks; boxes and wires on request.")
            .example("doc=quest:quests/wolf.nqasset boxes=true wires=true"),
        graph_get,
    );
    reg.cmd(
        Spec::view(
            "graph.find",
            "Finds nodes in a graph (the tab follows the results) or in every graph of the project.",
        )
        .example("doc=quest:quests/wolf.nqasset query=wolf go=first"),
        graph_find,
    );
    reg.topic(Topic {
        name: "graph-ops",
        summary: "The operations graph.edit takes",
        text: |_| OPS_TOPIC.to_owned(),
    });
}

/// What the area hangs on the editor: the `file.create` templates of a quest and a
/// behaviour graph, open documents following a rename, the build's check of the graphs, and the
/// unsaved documents.
pub fn install(editor: &Editor) {
    // the pickers' lists and the documents' are game.lookup's
    gz_game::Lookups::of(editor).provide(lookup_types(), lookup);
    let templates = AssetTemplates::of(editor);
    templates.provide(TemplateKind::Quest, |cx| {
        Ok(asset::quest_template(&text::id_from_name(cx.stem)).into_bytes())
    });
    templates.provide(TemplateKind::Behaviour, |cx| {
        Ok(asset::behaviour_template(&text::id_from_name(cx.stem)).into_bytes())
    });
    ProjectHooks::of(editor).on_renamed(|ed, from, to| {
        if let Some(session) = ed.project_opt()
            && let Some(svc) = NqService::get(&session)
        {
            svc.retarget(ed, from, to);
        }
    });
    // the graphs die with the project, the work in them does not: whatever is still unsaved when
    // nothing settled it is written first
    ProjectHooks::of(editor).on_before_close(|ed, session| async move {
        if let Some(svc) = NqService::get(&session) {
            let saved = svc.save_dirty(&ed).await;
            if saved > 0 {
                ed.log.info(format!("[nq] {saved} unsaved graph(s) saved before leaving the project"));
            }
        }
    });
    gz_build::BuildChecks::of(editor).add(Arc::new(check::GraphCheck));
    // the graphs as kinds of assets: asset.check gives their problems per file, positioned
    let kinds = gz_asset::Kinds::of(editor);
    for k in crate::kind::kinds() {
        kinds.add(k);
    }
    kinds.covers_build_stage(check::STAGE);
    editor.add_dirty_source(Arc::new(NqDirty));
}
