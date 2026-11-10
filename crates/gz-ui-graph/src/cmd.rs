//! `quest_view` (`UIQuestGraph::McpView`): opens (or brings up) the graph tab of a file and sets
//! its view; the answer describes the frames that show the change, in one call. And
//! `quest_open`, which brings the tab up too.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_nq::NqService;
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::AREA;
use crate::canvas::Canvas;
use crate::tab::{self, DrawerTab, GraphTab};

const NEED_PATH: &str = "missing 'path' (project-relative, e.g. quests/wolf_debt.nqasset or npc/ambush.behasset)";

fn refused(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `NqUtil::ArgString` read it: present means given, even empty.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn num(args: &Args, key: &str) -> Option<f64> {
    match args.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
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

/// `show` | `hide` | `toggle` (`ArgToggle`): the new value, `None` when not given.
fn toggle(args: &Args, key: &str, current: bool) -> Result<Option<bool>, CmdError> {
    let Some(v) = arg(args, key).filter(|v| !v.is_empty()) else { return Ok(None) };
    match v.as_str() {
        "show" => Ok(Some(true)),
        "hide" => Ok(Some(false)),
        "toggle" => Ok(Some(!current)),
        _ => Err(refused(format!("{key} must be \"show\", \"hide\" or \"toggle\""))),
    }
}

/// Brings the tab of an open document up (`OpenMcpWindow`).
async fn open_tab(ed: &Arc<Editor>, wire: &str, id: &str) -> Result<(), CmdError> {
    let mut a = Args::new();
    a.insert("action".into(), json!("open"));
    a.insert("id".into(), json!(id));
    a.insert("kind".into(), json!(id.split(':').next().unwrap_or("quest")));
    a.insert("path".into(), json!(wire));
    let reply = ed.call("ui_tab", a, ed.ui_origin()).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let why = reply.get("error").and_then(Value::as_str).unwrap_or("window not found");
    Err(CmdError::new(if why.contains("without a window") { "no_window" } else { "refused" }, why))
}

fn tab_of<'s>(shell: &'s mut gz_app::ShellState, id: &str) -> Result<&'s mut GraphTab, CmdError> {
    shell.document_mut(id).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<GraphTab>()).ok_or_else(|| refused("window not found"))
}

/// What the view of a tab is now (`AppendView`).
fn view_json(t: &GraphTab, ctx: &egui::Context, open_tabs: usize) -> Value {
    let ppp = ctx.pixels_per_point();
    let m = &t.mirror;
    let abs = t.handle().map(|d| d.path().abs.to_string_lossy().replace('\\', "/")).unwrap_or_else(|| t.wire().to_owned());
    let nodes = t.handle().map_or(0, |d| d.store.snapshot().graph.nodes.len());
    let drawer = t.drawer();
    let pane = |r: Option<egui::Rect>| r.filter(|r| r.width() > 0.0).map_or(Value::Null, |r| json!([r0(r.min.x), r0(r.min.y), r0(r.width()), r0(r.height())]));
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
    let mut o = Map::new();
    o.insert("path".into(), json!(abs));
    o.insert("zoom_level".into(), json!(m.zoom));
    o.insert("zoom".into(), r3(gz_nq::doc::zoom_of(m.zoom)));
    o.insert("ui_scale".into(), json!((f64::from(ppp) * 100.0).round() / 100.0));
    o.insert("title_px".into(), r0(Canvas::title_px(m, ppp)));
    o.insert("frame_min_level".into(), json!(gz_nq::doc::ZOOM_FRAME_MIN));
    o.insert("center".into(), json!([r1(m.center[0]), r1(m.center[1])]));
    o.insert("nodes".into(), json!(nodes));
    o.insert("open_tabs".into(), json!(open_tabs));
    o.insert("slot".into(), json!(m.slot));
    o.insert("selected".into(), json!(m.selection));
    o.insert("keys".into(), json!(t.show_keys));
    o.insert("problems".into(), json!(drawer == DrawerTab::Problems));
    o.insert("drawer".into(), json!(drawer.name()));
    o.insert("graph_panel".into(), json!(t.graph_panel_state()));
    o.insert("details".into(), json!(t.show_details));
    o.insert("minimap".into(), json!(t.canvas.minimap_shown));
    let b = t.canvas.beyond;
    o.insert("beyond".into(), json!({ "above": b[0], "below": b[1], "left": b[2], "right": b[3] }));
    o.insert("graph_width".into(), r0(t.graph_width(ctx)));
    o.insert("details_width".into(), r0(t.details_width(ctx)));
    o.insert("panes".into(), Value::Object(panes));
    o.insert("action_split".into(), r3(ins.action_share));
    o.insert("vars_split".into(), r3(ins.vars_share));
    o.insert("node_scroll".into(), scroll(ins.node_scroll));
    o.insert("action_scroll".into(), scroll(ins.action_scroll));
    o.insert("graph_scroll".into(), scroll(ins.graph_scroll));
    o.insert("variables_scroll".into(), scroll(ins.vars_scroll));
    o.insert("pending".into(), json!(t.view_pending()));
    Value::Object(o)
}

fn graph_tabs(shell: &gz_app::ShellState) -> usize {
    shell
        .tabs_json()
        .iter()
        .filter(|t| t.get("id").and_then(Value::as_str).is_some_and(|i| i.starts_with("quest:") || i.starts_with("beh:")))
        .count()
}

/// What quest_view asks of the view, read before the UI thread applies it.
struct ViewReq {
    frame: String,
    zoom: Option<i32>,
    center: Option<[f32; 2]>,
    slot: String,
    select: String,
    scroll: String,
    graph_width: Option<f64>,
    details_width: Option<f64>,
    action_split: Option<f64>,
    vars_split: Option<f64>,
    args: Args,
}

/// Applies a quest_view to its tab (`McpView`): the frames it owes.
#[allow(clippy::too_many_lines)]
fn apply(t: &mut GraphTab, ctx: &egui::Context, r: ViewReq) -> Result<u32, CmdError> {
    let mut asked = !t.drawn_lately(ctx);
    // the panes first: a scroll asked for in the same call is judged against them
    if let Some(v) = toggle(&r.args, "graph_panel", t.show_graph && t.graph_panel_state() != "rail")? {
        t.set_graph_panel(v);
        asked = true;
    }
    if let Some(v) = toggle(&r.args, "details", t.show_details)? {
        t.show_details = v;
        asked = true;
    }
    if let Some(v) = toggle(&r.args, "keys", t.show_keys)? {
        t.show_keys = v;
        asked = true;
    }
    // the drawer: problems=show opens it on the problems, find results come through quest_find
    let drawer = t.drawer();
    if let Some(v) = toggle(&r.args, "problems", drawer == DrawerTab::Problems)? {
        let next = if v {
            DrawerTab::Problems
        } else if drawer == DrawerTab::Problems {
            DrawerTab::None
        } else {
            drawer
        };
        t.set_drawer(ctx, next);
        asked = true;
    }
    if let Some(w) = r.graph_width {
        gz_ui::layout::set(ctx, tab::GRAPH_W, (w as f32).clamp(tab::GRAPH_MIN, tab::GRAPH_MAX));
        asked = true;
    }
    if let Some(w) = r.details_width {
        gz_ui::layout::set(ctx, tab::DETAILS_W, (w as f32).clamp(tab::DETAILS_MIN, tab::DETAILS_MAX));
        asked = true;
    }
    if r.action_split.is_some() || r.vars_split.is_some() {
        t.inspector.set_split(ctx, r.action_split.filter(|v| *v >= 0.0).map(|v| v as f32), r.vars_split.filter(|v| *v >= 0.0).map(|v| v as f32));
        asked = true;
    }
    let graph = t.handle().map(|d| d.store.snapshot()).ok_or_else(|| refused("window not found"))?;
    let mut want: Vec<SmolStr> = Vec::new();
    if !r.select.is_empty() && r.select != "none" {
        for id in r.select.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if graph.graph.find_node(id).is_none() {
                return Err(refused(format!("no node '{id}' in this graph")));
            }
            if !want.iter().any(|w| w == id) {
                want.push(SmolStr::new(id));
            }
        }
        if want.is_empty() {
            return Err(refused("select needs a node id, a comma separated list, or \"none\""));
        }
    }
    // validator-only slots are refused before any part of the view changes
    if !r.slot.is_empty() && r.slot != "none" {
        let digits = r.slot.strip_prefix("enter:").or_else(|| r.slot.strip_prefix("exit:"));
        let ok = digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        if !ok {
            return Err(refused("slot must be \"enter:N\", \"exit:N\" or \"none\""));
        }
    }
    if !r.frame.is_empty() {
        if r.frame == "all" {
            t.canvas.request_frame_all();
        } else if graph.graph.find_node(&r.frame).is_some() {
            t.frame_node(ctx, &r.frame);
        }
    }
    if let Some(z) = r.zoom {
        // an asked-for zoom outranks the zoom a pending frame would pick
        t.mirror.zoom = z.clamp(0, gz_nq::doc::ZOOM.len() as i32 - 1);
        t.canvas.want_zoom = Some(t.mirror.zoom);
    }
    if let Some(c) = r.center {
        t.canvas.cancel_framing();
        t.mirror.center = [gz_nq::layout::sane(c[0]), gz_nq::layout::sane(c[1])];
    }
    // before the slot: selecting clears it, and "select a node, open its action" ends on the action
    if !r.select.is_empty() {
        t.mirror.selection = want;
        t.mirror.slot.clear();
    }
    if !r.slot.is_empty() {
        t.mirror.slot = if r.slot == "none" { String::new() } else { r.slot.clone() };
    }
    if !r.scroll.is_empty() {
        t.can_scroll(&r.scroll).map_err(|why| refused(format!("scroll={}: {why}", r.scroll)))?;
        t.inspector.scroll = Some(r.scroll.clone());
    }
    let any = asked || !r.frame.is_empty() || r.zoom.is_some() || r.center.is_some() || !r.select.is_empty() || !r.slot.is_empty() || !r.scroll.is_empty();
    let owe = if any { if !r.scroll.is_empty() || !r.slot.is_empty() { 2 } else { 1 } } else { 0 };
    t.owe_frames(owe);
    ctx.request_repaint();
    Ok(owe)
}

async fn quest_view(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let Some(path) = arg(&args, "path").filter(|p| !p.is_empty()) else { return Err(refused("path is required")) };
    let svc = NqService::of(&ed)?;
    let d = svc.open(&ed, &path).await.map_err(refused)?;
    let wire = d.path().wire;
    let id = tab::doc_id(&wire, gz_nq::asset::is_behaviour_path(&wire));
    open_tab(&ed, &wire, &id).await?;
    if args.flag("answer_only") {
        let id2 = id.clone();
        return gz_app::on_ui(&ed, move |shell, ctx| {
            let n = graph_tabs(shell);
            let t = tab_of(shell, &id2)?;
            Ok(view_json(t, ctx, n))
        })
        .await;
    }
    let zoom = num(&args, "zoom_level").map(|z| z as i32).filter(|z| *z >= 0);
    let center = match (num(&args, "cx"), num(&args, "cy")) {
        (Some(x), Some(y)) => Some([x as f32, y as f32]),
        _ => None,
    };
    let req = ViewReq {
        frame: arg(&args, "frame").unwrap_or_default(),
        zoom,
        center,
        slot: arg(&args, "slot").unwrap_or_default(),
        select: arg(&args, "select").unwrap_or_default(),
        scroll: arg(&args, "scroll").unwrap_or_default(),
        graph_width: num(&args, "graph_width"),
        details_width: num(&args, "details_width"),
        action_split: num(&args, "action_split"),
        vars_split: num(&args, "vars_split"),
        args,
    };
    let id2 = id.clone();
    let owe = gz_app::on_ui(&ed, move |shell, ctx| {
        let t = tab_of(shell, &id2)?;
        apply(t, ctx, req)
    })
    .await?;
    // the answer is the last frame drawn: held until the frames this call changed are drawn
    if owe > 0 {
        ed.frames.after(u64::from(owe)).await?;
        for _ in 0..30 {
            let id3 = id.clone();
            let pending = gz_app::on_ui(&ed, move |shell, _| Ok(tab_of(shell, &id3)?.view_pending())).await?;
            if !pending {
                break;
            }
            ed.frames.after(1).await?;
        }
    }
    gz_app::on_ui(&ed, move |shell, ctx| {
        let n = graph_tabs(shell);
        let t = tab_of(shell, &id)?;
        Ok(view_json(t, ctx, n))
    })
    .await
}

/// `quest_open` with the tab: the document opens, its tab comes up when there is a window, and
/// the answer is the document's as `gz_nq` gives it (the open document's summary and outline).
async fn quest_open(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let Some(path) = arg(&args, "path").filter(|p| !p.is_empty()) else { return Err(refused(NEED_PATH)) };
    let svc = NqService::of(&ed)?;
    let d = svc.open(&ed, &path).await.map_err(refused)?;
    if ed.frames.has_window() {
        let wire = d.path().wire;
        let id = tab::doc_id(&wire, gz_nq::asset::is_behaviour_path(&wire));
        if let Err(e) = open_tab(&ed, &wire, &id).await {
            ed.log.warn(format!("[nq] the tab of {wire} did not come up: {}", e.message));
        }
    }
    // an open document answers quest_validate with exactly what quest_open answers
    let reply = ed.call("quest_validate", args, o).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(reply);
    }
    Err(refused(reply.get("error").and_then(Value::as_str).unwrap_or("quest_open failed")))
}

/// Registers `quest_view` and `quest_open` (after `gz_nq::register`: this one brings the tab up).
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "quest_view", quest_view);
    reg.add(AREA, "quest_open", quest_open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles() {
        let mut a = Args::new();
        a.insert("details".into(), json!("toggle"));
        a.insert("keys".into(), json!("sideways"));
        assert_eq!(toggle(&a, "details", true).ok().flatten(), Some(false));
        assert_eq!(toggle(&a, "graph_panel", true).ok().flatten(), None);
        assert_eq!(toggle(&a, "keys", false).err().map(|e| e.message), Some("keys must be \"show\", \"hide\" or \"toggle\"".to_owned()));
        assert_eq!(tab::doc_id("quests\\a.nqasset", false), "quest:quests/a.nqasset");
        assert_eq!(tab::doc_id("npc/x.behasset", true), "beh:npc/x.behasset");
    }
}
