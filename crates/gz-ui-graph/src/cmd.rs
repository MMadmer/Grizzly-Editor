//! `graph.view`: brings the tab of a graph up and sets its view - the zoom, the centre, the
//! selection, the panels and their splits, a menu or the picker up to look at, the drawer - and
//! answers what the frames that show the change drew, in one call.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, CmdError, Editor, Registry, Reply, Spec};
use gz_nq::NqService;
use gz_nq::doc::{Grow, ViewOp};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::canvas::Canvas;
use crate::tab::{self, DrawerTab, GraphTab};

fn refused(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

fn r1(v: f32) -> Value {
    json!((f64::from(v) * 10.0).round() / 10.0)
}

fn r0(v: f32) -> Value {
    json!(f64::from(v).round())
}

fn r3(v: f32) -> Value {
    json!((f64::from(v) * 1000.0).round() / 1000.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Shown {
    Show,
    Hide,
    Toggle,
}

impl Shown {
    fn apply(self, now: bool) -> bool {
        match self {
            Shown::Show => true,
            Shown::Hide => false,
            Shown::Toggle => !now,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum GrowWhat {
    Connected,
    Upstream,
    Downstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum HistoryWhat {
    Back,
    Forward,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DrawerWhat {
    None,
    Problems,
    Find,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MenuWhat {
    Node,
    Wire,
    Canvas,
    Frame,
    Picker,
    Replace,
    None,
}

impl MenuWhat {
    fn name(self) -> &'static str {
        match self {
            MenuWhat::Node => "node",
            MenuWhat::Wire => "wire",
            MenuWhat::Canvas => "canvas",
            MenuWhat::Frame => "frame",
            MenuWhat::Picker => "picker",
            MenuWhat::Replace => "replace",
            MenuWhat::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FocusWhat {
    Rename,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WireWhat {
    RemoveHovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DialogWhat {
    FindInProject,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// The graph: its tab (`quest:<path>`, `beh:<path>`) or its file, project-relative. Left out:
    /// the graph tab that is up.
    #[serde(default)]
    doc: Option<String>,
    /// Frames the whole graph (`all`), the selection (`selection`) or one node (its id).
    #[serde(default)]
    frame: Option<String>,
    /// The zoom level, 0 (5%) to 20 (200%); 16 is 1:1.
    #[serde(default)]
    zoom_level: Option<i32>,
    /// The zoom as a factor (0.05 to 2): the nearest level at or under it.
    #[serde(default)]
    zoom: Option<f32>,
    /// The world point at the view's centre, `[x, y]` in pt.
    #[serde(default)]
    center: Option<[f32; 2]>,
    /// The nodes to select: a list of ids, `all` or `none`.
    #[serde(default)]
    select: Option<Value>,
    /// The selection grows along the wires: connected, upstream or downstream.
    #[serde(default)]
    grow: Option<GrowWhat>,
    /// The action the Details panel opens: `enter:N`, `exit:N` (0-based) or `none`.
    #[serde(default)]
    slot: Option<String>,
    /// Back or forward through the view history, or clear it.
    #[serde(default)]
    history: Option<HistoryWhat>,
    /// Goes to a bookmark: its name or number (1-based), `next` or `previous`.
    #[serde(default)]
    bookmark: Option<String>,
    /// The drawer under the panes: none, problems or find (its field takes the keys).
    #[serde(default)]
    drawer: Option<DrawerWhat>,
    /// The Graph panel (header, tasks or roles, variables): show, hide or toggle.
    #[serde(default)]
    graph_panel: Option<Shown>,
    /// The Details panel: show, hide or toggle. Folded to a rail, show opens it over the canvas.
    #[serde(default)]
    details: Option<Shown>,
    /// The Keyboard Shortcuts card.
    #[serde(default)]
    keys: Option<Shown>,
    /// The minimap of a graph big enough for one.
    #[serde(default)]
    minimap: Option<Shown>,
    /// The folded More of a behaviour graph's roles (title, binding, default).
    #[serde(default)]
    roles_more: Option<Shown>,
    /// The Graph panel's width, pt.
    #[serde(default)]
    graph_width: Option<f32>,
    /// The Details panel's width, pt.
    #[serde(default)]
    details_width: Option<f32>,
    /// The share of the Details panel the open action takes (0-1).
    #[serde(default)]
    action_split: Option<f32>,
    /// Scrolls a pane: top, bottom, action:top, action:bottom, tasks, task:<id>, task:<id>/steps
    /// or variables.
    #[serde(default)]
    scroll: Option<String>,
    /// Puts a menu up to look at: a node's (the selected one), a wire's, the canvas's, a frame's,
    /// the picker, the picker of Replace Node; none closes it.
    #[serde(default)]
    menu: Option<MenuWhat>,
    /// With menu=picker: the picker of a wire dragged out of this output (`node.pin`) and dropped
    /// beside it - only kinds that take an input; the node picked is wired to it.
    #[serde(default)]
    from_pin: Option<String>,
    /// With menu=picker: the picker of a wire dragged into this node's input - only kinds that go
    /// on.
    #[serde(default)]
    into: Option<String>,
    /// rename: the Details panel's id field takes the keys, as F2 does.
    #[serde(default)]
    focus: Option<FocusWhat>,
    /// remove_hovered: the wire under the pointer goes, as D does.
    #[serde(default)]
    wire: Option<WireWhat>,
    /// find_in_project: the Find in Project dialog with the graph's search.
    #[serde(default)]
    dialog: Option<DialogWhat>,
    /// Answers what the tab shows now, changing nothing and waiting for no frame.
    #[serde(default)]
    answer_only: bool,
}

/// Brings the tab of an open document up.
async fn open_tab(ed: &Arc<Editor>, wire: &str, id: &str) -> Result<(), CmdError> {
    let mut a = Args::new();
    a.insert("action".into(), json!("open"));
    a.insert("doc".into(), json!(id));
    a.insert("kind".into(), json!(id.split(':').next().unwrap_or("quest")));
    a.insert("path".into(), json!(wire));
    let reply = ed.call("ui.tab", a, ed.ui_origin()).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let why = reply.get("error").and_then(Value::as_str).unwrap_or("window not found");
    Err(CmdError::new(if why.contains("without a window") { "no_window" } else { "refused" }, why))
}

fn tab_of<'s>(shell: &'s mut gz_app::ShellState, id: &str) -> Result<&'s mut GraphTab, CmdError> {
    shell
        .document_mut(id)
        .and_then(|d| d.as_any_mut())
        .and_then(|a| a.downcast_mut::<GraphTab>())
        .ok_or_else(|| refused(format!("'{id}' is no open graph (ui.tab lists the tabs)")))
}

/// What the view of a tab is now.
fn view_json(t: &GraphTab, ctx: &egui::Context) -> Value {
    let ppp = ctx.pixels_per_point();
    let m = &t.mirror;
    let snap = t.handle().map(|d| d.store.snapshot());
    let nodes = snap.as_ref().map_or(0, |s| s.graph.nodes.len());
    let drawer = t.drawer();
    let pane = |r: Option<egui::Rect>| {
        r.filter(|r| r.width() > 0.0)
            .map_or(Value::Null, |r| json!([r0(r.min.x), r0(r.min.y), r0(r.width()), r0(r.height())]))
    };
    let graph_drawn = t.pane_graph.is_some();
    let details_drawn = t.pane_details.is_some();
    let ins = &t.inspector;
    let scroll = |s: [f32; 2]| json!([r0(s[0]), r0(s[1])]);
    let mut panes = Map::new();
    panes.insert("graph".into(), pane(t.pane_graph));
    panes.insert("variables".into(), if graph_drawn { pane(ins.vars_pane) } else { Value::Null });
    panes.insert("canvas".into(), pane(t.pane_canvas));
    panes.insert("details".into(), pane(t.pane_details));
    panes.insert("node".into(), if details_drawn { pane(ins.node_pane) } else { Value::Null });
    panes.insert("action".into(), if details_drawn { pane(ins.action_pane) } else { Value::Null });
    panes.insert("drawer".into(), pane(t.pane_drawer));
    let set = t.wires_in_force();
    let mut o = Map::new();
    o.insert("doc".into(), json!(t.doc_id()));
    o.insert("path".into(), json!(t.wire()));
    o.insert("zoom_level".into(), json!(m.zoom));
    o.insert("zoom".into(), r3(gz_nq::doc::zoom_of(m.zoom)));
    o.insert("ui_scale".into(), json!((f64::from(ppp) * 100.0).round() / 100.0));
    o.insert("title_px".into(), r0(Canvas::title_px(m, ppp)));
    o.insert("detail".into(), json!(t.canvas.drawn_detail.name()));
    o.insert("center".into(), json!([r1(m.center[0]), r1(m.center[1])]));
    o.insert("nodes".into(), json!(nodes));
    o.insert("flow".into(), json!(snap.as_ref().map_or("right", |s| s.graph.flow().key())));
    o.insert(
        "wire_style".into(),
        json!({
            "style": set.style.key(), "diagonal": set.diagonal.key(), "thickness": set.thickness,
            "by_pin": set.by_pin, "dim": set.dim, "bubbles": set.bubbles, "arrows": set.arrows,
            "own": snap.as_ref().is_some_and(|s| s.graph.canvas.wires.style.is_some()),
        }),
    );
    o.insert("slot".into(), json!(m.slot));
    o.insert("selected".into(), json!(m.selection));
    o.insert("keys".into(), json!(t.show_keys));
    o.insert("drawer".into(), json!(drawer.name()));
    o.insert("graph_panel".into(), json!(t.graph_panel_state()));
    o.insert("details".into(), json!(t.details_state()));
    o.insert("minimap".into(), json!(t.canvas.minimap_shown));
    o.insert("menu".into(), json!(t.canvas.menu_up(ctx)));
    o.insert(
        "picker".into(),
        t.canvas.picker_info(ctx).map_or(
            Value::Null,
            |(what, kinds)| json!({ "for": what, "count": kinds.len(), "kinds": kinds }),
        ),
    );
    o.insert(
        "bookmarks".into(),
        json!(snap.as_ref().map_or_else(Vec::new, |s| {
            s.graph
                .canvas
                .bookmarks
                .iter()
                .map(|b| json!({ "name": b.name, "node": b.node }))
                .collect::<Vec<_>>()
        })),
    );
    o.insert(
        "history".into(),
        json!({ "back": snap.as_ref().map_or(0, |s| s.view.back.len()), "forward": snap.as_ref().map_or(0, |s| s.view.forward.len()) }),
    );
    let b = t.canvas.beyond;
    o.insert("beyond".into(), json!({ "above": b[0], "below": b[1], "left": b[2], "right": b[3] }));
    o.insert("graph_width".into(), r0(t.graph_width(ctx)));
    o.insert("details_width".into(), r0(t.details_width(ctx)));
    o.insert("panes".into(), Value::Object(panes));
    o.insert("action_split".into(), r3(ins.action_share));
    o.insert("vars_share".into(), r3(ins.vars_share));
    o.insert("node_scroll".into(), scroll(ins.node_scroll));
    o.insert("action_scroll".into(), scroll(ins.action_scroll));
    o.insert("graph_scroll".into(), scroll(ins.graph_scroll));
    o.insert("variables_scroll".into(), scroll(ins.vars_scroll));
    o.insert("pending".into(), json!(t.view_pending()));
    Value::Object(o)
}

/// Applies a `graph.view` to its tab: the frames it owes before its answer describes it.
#[allow(clippy::too_many_lines)]
fn apply(t: &mut GraphTab, ctx: &egui::Context, p: &ViewParams) -> Result<u32, CmdError> {
    let mut owe = u32::from(!t.drawn_lately(ctx));
    let mut asked = |n: u32| owe = owe.max(n);
    // the panes first: a scroll asked for in the same call is judged against them
    if let Some(v) = p.graph_panel {
        t.set_graph_panel(v.apply(t.graph_on_screen()));
        asked(1);
    }
    if let Some(v) = p.details {
        let on = v.apply(t.show_details && (!t.details_rail || t.details_overlay));
        t.show_details = on || t.details_rail;
        t.details_overlay = on && t.details_rail;
        if !on && !t.details_rail {
            t.show_details = false;
        }
        asked(1);
    }
    if let Some(v) = p.keys {
        t.show_keys = v.apply(t.show_keys);
        asked(1);
    }
    if let Some(v) = p.roles_more {
        t.inspector.set_roles_more(ctx, v.apply(false));
        asked(1);
    }
    if let Some(v) = p.minimap {
        let now = t.handle().is_some_and(|d| d.store.snapshot().view.minimap);
        t.post_view(ctx, ViewOp::Minimap(v.apply(now)));
        asked(1);
    }
    if let Some(d) = p.drawer {
        match d {
            DrawerWhat::None => t.set_drawer(ctx, DrawerTab::None),
            DrawerWhat::Problems => t.set_drawer(ctx, DrawerTab::Problems),
            DrawerWhat::Find => t.open_find(ctx),
        }
        asked(1);
    }
    if let Some(w) = p.graph_width {
        gz_ui::layout::set(ctx, tab::GRAPH_W, w.clamp(tab::GRAPH_MIN, tab::GRAPH_MAX));
        asked(1);
    }
    if let Some(w) = p.details_width {
        gz_ui::layout::set(ctx, tab::DETAILS_W, w.clamp(tab::DETAILS_MIN, tab::DETAILS_MAX));
        asked(1);
    }
    if let Some(f) = p.action_split.filter(|v| *v >= 0.0) {
        t.inspector.set_split(ctx, Some(f));
        asked(1);
    }
    let snap =
        t.handle().map(|d| d.store.snapshot()).ok_or_else(|| refused("the graph is still being read"))?;
    let g = &snap.graph;
    // what is refused is refused before any part of the view changes
    let mut want: Option<Vec<SmolStr>> = None;
    match &p.select {
        None => {}
        Some(Value::String(s)) if s == "none" => want = Some(Vec::new()),
        Some(Value::String(s)) if s == "all" => want = Some(g.nodes.iter().map(|n| n.id.clone()).collect()),
        Some(v) => {
            let ids: Vec<String> = match v {
                Value::String(s) => {
                    s.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect()
                }
                Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                _ => return Err(CmdError::invalid("select is a list of node ids, \"all\" or \"none\"")),
            };
            let mut list = Vec::new();
            for id in ids {
                if g.find_node(&id).is_none() {
                    return Err(refused(format!("no node '{id}' in this graph")));
                }
                if !list.iter().any(|w: &SmolStr| *w == id) {
                    list.push(SmolStr::new(id));
                }
            }
            want = Some(list);
        }
    }
    if let Some(slot) = p.slot.as_deref().filter(|s| *s != "none") {
        let digits = slot.strip_prefix("enter:").or_else(|| slot.strip_prefix("exit:"));
        if !digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) {
            return Err(CmdError::invalid("slot is \"enter:N\", \"exit:N\" or \"none\""));
        }
    }
    if let Some(f) = p.frame.as_deref().filter(|f| *f != "all" && *f != "selection")
        && g.find_node(f).is_none()
    {
        return Err(refused(format!("no node '{f}' in this graph")));
    }
    match p.frame.as_deref() {
        Some("all") => t.canvas.request_frame_all(),
        Some("selection") => t.canvas.request_frame_selection(),
        Some(id) => t.frame_node(ctx, id),
        None => {}
    }
    if let Some(z) = p.zoom_level.or_else(|| p.zoom.map(gz_nq::doc::level_at_most)) {
        // an asked-for zoom outranks the zoom a pending frame would pick
        t.mirror.zoom = z.clamp(0, gz_nq::doc::ZOOM.len() as i32 - 1);
        t.canvas.want_zoom = Some(t.mirror.zoom);
    }
    if let Some(c) = p.center {
        t.canvas.cancel_framing();
        t.mirror.center = [gz_nq::layout::sane(c[0]), gz_nq::layout::sane(c[1])];
    }
    // before the slot: selecting clears it, and "select a node, open its action" ends on the action
    if let Some(w) = want {
        t.mirror.selection = w;
        t.mirror.slot.clear();
    }
    if let Some(gw) = p.grow {
        let dir = match gw {
            GrowWhat::Connected => Grow::Connected,
            GrowWhat::Upstream => Grow::Upstream,
            GrowWhat::Downstream => Grow::Downstream,
        };
        t.mirror.selection = gz_nq::doc::grown(g, &t.mirror.selection, dir);
        t.mirror.slot.clear();
    }
    if let Some(slot) = &p.slot {
        t.mirror.slot = if slot == "none" { String::new() } else { slot.clone() };
    }
    if let Some(h) = p.history {
        t.post_view(
            ctx,
            match h {
                HistoryWhat::Back => ViewOp::HistoryBack,
                HistoryWhat::Forward => ViewOp::HistoryForward,
                HistoryWhat::Clear => ViewOp::HistoryClear,
            },
        );
        asked(2);
    }
    if let Some(b) = p.bookmark.as_deref() {
        if g.canvas.bookmarks.is_empty() {
            return Err(refused("the graph has no bookmark (Ctrl+Shift+1 sets one)"));
        }
        t.post_view(
            ctx,
            match b {
                "next" => ViewOp::BookmarkJump(1),
                "previous" => ViewOp::BookmarkJump(-1),
                name => ViewOp::BookmarkGo(name.to_owned()),
            },
        );
        asked(2);
    }
    if let Some(s) = &p.scroll {
        t.can_scroll(s).map_err(|why| refused(format!("scroll={s}: {why}")))?;
        t.inspector.scroll = Some(s.clone());
        asked(2);
    }
    if p.focus == Some(FocusWhat::Rename) {
        if t.mirror.selection.len() != 1 {
            return Err(refused("select the one node to rename"));
        }
        t.canvas.want_rename = true;
        asked(2);
    }
    if p.wire == Some(WireWhat::RemoveHovered) && !t.remove_hovered_wire(ctx) {
        return Err(refused("no wire is under the pointer"));
    }
    if p.dialog == Some(DialogWhat::FindInProject) {
        t.open_project_find(ctx);
        asked(2);
    }
    if let Some(m) = p.menu {
        let wired = p.from_pin.is_some() || p.into.is_some();
        if m == MenuWhat::Picker && wired {
            let from = p.from_pin.as_deref().map(|f| f.split_once('.').unwrap_or((f, "next")));
            t.open_picker_wired(ctx, from, p.into.as_deref()).map_err(refused)?;
        } else if wired {
            return Err(CmdError::invalid("from_pin and into go with menu=picker"));
        } else {
            t.open_menu(ctx, m.name()).map_err(refused)?;
        }
        asked(2);
    }
    if p.frame.is_some()
        || p.zoom_level.is_some()
        || p.zoom.is_some()
        || p.center.is_some()
        || p.select.is_some()
        || p.grow.is_some()
    {
        asked(1);
    }
    if p.slot.is_some() {
        asked(2);
    }
    t.owe_frames(owe);
    ctx.request_repaint();
    Ok(owe)
}

async fn graph_view(ed: Arc<Editor>, p: ViewParams, _o: Origin) -> Reply {
    let svc = NqService::of(&ed)?;
    // the graph named, or the graph tab that is up
    let id = match p.doc.as_deref().filter(|d| !d.is_empty()) {
        Some(doc) => {
            let d = svc.open(&ed, gz_nq::doc_path(doc)).await.map_err(|e| {
                if e.starts_with("file does not exist") { CmdError::not_found(e) } else { refused(e) }
            })?;
            let wire = d.path().wire;
            let id = gz_nq::doc_id(&wire);
            open_tab(&ed, &wire, &id).await?;
            id
        }
        None => {
            if !ed.frames.has_window() {
                return Err(CmdError::new(
                    "no_window",
                    "graph.view needs the editor's window; graph.get reads a graph without one",
                ));
            }
            gz_app::on_ui(&ed, |shell, _| {
                let a = shell.active().to_owned();
                if a.starts_with("quest:") || a.starts_with("beh:") {
                    Ok(a)
                } else {
                    Err(CmdError::invalid("no graph tab is up: pass doc"))
                }
            })
            .await?
        }
    };
    if p.answer_only {
        let id2 = id.clone();
        return gz_app::on_ui(&ed, move |shell, ctx| Ok(view_json(tab_of(shell, &id2)?, ctx))).await;
    }
    let id2 = id.clone();
    let p = Arc::new(p);
    let p2 = p.clone();
    let owe = gz_app::on_ui(&ed, move |shell, ctx| apply(tab_of(shell, &id2)?, ctx, &p2)).await?;
    // the answer is the last frame drawn: held until the frames this call changed are drawn, and
    // until a tab that came up a moment ago (its graph still being read) was drawn at all
    if owe > 0 {
        ed.frames.after(u64::from(owe)).await?;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let id3 = id.clone();
        let pending = gz_app::on_ui(&ed, move |shell, _| Ok(tab_of(shell, &id3)?.view_pending())).await?;
        if !pending || std::time::Instant::now() > deadline {
            break;
        }
        ed.frames.after(1).await?;
    }
    gz_app::on_ui(&ed, move |shell, ctx| Ok(view_json(tab_of(shell, &id)?, ctx))).await
}

/// Registers `graph.view` (after `gz_nq::register`, which adds the area and the other `graph.*`).
pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::view(
            "graph.view",
            "Brings a graph's tab up and sets its view: zoom, centre, selection, panels, a menu or the picker.",
        )
        .example("doc=quest:quests/a.nqasset frame=all")
        .example("menu=picker"),
        graph_view,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_ids() {
        assert!(Shown::Show.apply(false));
        assert!(!Shown::Hide.apply(true));
        assert!(Shown::Toggle.apply(false));
        assert_eq!(tab::doc_id("quests\\a.nqasset", false), "quest:quests/a.nqasset");
        assert_eq!(tab::doc_id("npc/x.behasset", true), "beh:npc/x.behasset");
        let p: ViewParams =
            serde_json::from_value(json!({ "menu": "picker", "select": ["a"] })).unwrap_or_default();
        assert_eq!(p.menu, Some(MenuWhat::Picker));
        assert!(serde_json::from_value::<ViewParams>(json!({ "sideways": true })).is_err());
    }
}
