//! `viewport_gesture`: a gesture of the mouse - a click, a double click, a drag, a hover - fed to
//! the viewport over MCP. It enters where the window's input enters the pane (`ViewportPane::
//! input`), one step a frame, so the camera, the gizmo, the tools' presses, strokes and marquees
//! take it exactly as they take the mouse; nothing is sent to the operating system and no code
//! path of its own decides what it does.
//!
//! The command answers once every step was taken, the work the steps started has ended and the
//! frames that show it were drawn: what was picked, placed, moved or removed, by name.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use egui::{Pos2, pos2};
use glam::Vec3;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Reply, Value, json};
use gz_scene::{ObjData, Object, SceneService, SceneState};

use crate::gizmo::{self, GizmoType};
use crate::service::{Action, Viewport};

/// One frame of a gesture: where the pointer is (points from the pane's top left) and the
/// buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Step {
    pub at: Pos2,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub pressed: bool,
    pub released: bool,
}

/// The keys held through a gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A gesture waiting for the pane.
pub(crate) struct Pending {
    steps: VecDeque<Step>,
    pub mods: Mods,
    done: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Viewport {
    /// The step the pane takes this frame, and whether it was the last.
    pub(crate) fn gesture_step(&self) -> Option<(Step, Mods, bool)> {
        let mut g = self.gesture.lock();
        let p = g.as_mut()?;
        let step = p.steps.pop_front()?;
        let mods = p.mods;
        let last = p.steps.is_empty();
        if last && let Some(done) = g.take().and_then(|p| p.done) {
            let _ = done.send(());
        }
        Some((step, mods, last))
    }
}

/// Starts a task of a gesture: `viewport_gesture` answers only once it has ended.
pub(crate) fn spawn<F>(editor: &Editor, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let vp = Viewport::of(editor);
    vp.tracked.fetch_add(1, Ordering::AcqRel);
    let held = Tracked(vp);
    editor.exec.spawn(async move {
        let _held = held;
        fut.await;
    });
}

struct Tracked(Arc<Viewport>);

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.tracked.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A point of the pane: `[x, y]`, `"x,y"` or an object `{x, y}`, in points.
fn point(a: &Args, key: &str) -> Result<Option<Pos2>, CmdError> {
    let bad = || CmdError::invalid(format!("'{key}' must be [x, y] in viewport points"));
    let Some(v) = a.get(key) else { return Ok(None) };
    let num = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
    let (x, y) = match v {
        Value::Array(xs) if xs.len() == 2 => (num(&xs[0]), num(&xs[1])),
        Value::String(s) => {
            let mut parts = s.split(',');
            (
                parts.next().and_then(|p| p.trim().parse().ok()),
                parts.next().and_then(|p| p.trim().parse().ok()),
            )
        }
        Value::Object(m) => (m.get("x").and_then(num), m.get("y").and_then(num)),
        _ => return Err(bad()),
    };
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Some(pos2(x as f32, y as f32))),
        _ => Err(bad()),
    }
}

fn mods_of(a: &Args) -> Result<Mods, CmdError> {
    let mut m = Mods::default();
    let names: Vec<String> = match a.get("mods") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => s.split([',', '+', ' ']).map(str::to_owned).collect(),
        Some(Value::Array(xs)) => xs.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        Some(_) => return Err(CmdError::invalid("'mods' must name ctrl, shift or alt")),
    };
    for n in names.iter().map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()) {
        match n.as_str() {
            "ctrl" | "control" => m.ctrl = true,
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            other => return Err(CmdError::invalid(format!("unknown modifier '{other}' (ctrl, shift, alt)"))),
        }
    }
    Ok(m)
}

/// The frames of a gesture.
fn steps_of(kind: &str, at: Pos2, to: Pos2, button: &str, moves: usize) -> VecDeque<Step> {
    let down = |pressed: bool, at: Pos2| Step {
        at,
        left: button == "left",
        right: button == "right",
        middle: button == "middle",
        pressed,
        released: false,
    };
    let up = |at: Pos2| Step { at, released: true, ..Step::default() };
    let rest = |at: Pos2| Step { at, ..Step::default() };
    let mut s = VecDeque::new();
    // the pointer arrives first: the gizmo's part under it is found before the press, as for
    // the mouse
    s.push_back(rest(at));
    match kind {
        "hover" => {}
        "click" => {
            s.push_back(down(true, at));
            s.push_back(up(at));
        }
        "double_click" => {
            for _ in 0..2 {
                s.push_back(down(true, at));
                s.push_back(up(at));
            }
        }
        _ => {
            s.push_back(down(true, at));
            for i in 1..=moves {
                let t = i as f32 / moves as f32;
                s.push_back(down(false, at + (to - at) * t));
            }
            s.push_back(up(to));
        }
    }
    s.push_back(rest(if kind == "drag" { to } else { at }));
    s
}

fn v3(v: Vec3) -> Value {
    let r = |x: f32| (f64::from(x) * 1000.0).round() / 1000.0;
    json!([r(v.x), r(v.y), r(v.z)])
}

fn placed_json(o: &Object) -> Value {
    let mut m = json!({
        "name": o.name.as_str(),
        "class": o.class.name(),
        "position": v3(o.position()),
        "rotation": v3(o.rotation()),
        "scale": v3(o.scale()),
    });
    if let (Some(r), Value::Object(m)) = (o.ref_name(), &mut m) {
        m.insert("reference".into(), json!(r));
    }
    m
}

fn names(state: &SceneState, ids: impl Iterator<Item = gz_scene::ObjId>) -> Vec<Value> {
    ids.filter_map(|id| state.get(id)).map(|o| json!(o.name.as_str())).collect()
}

fn attached(o: &Object) -> Option<&str> {
    match &o.data {
        ObjData::Spawn(s) => s.attached.as_ref().map(|a| a.name.as_str()),
        _ => None,
    }
}

/// What a gesture changed in the scene, by name.
fn scene_diff(before: &SceneState, after: &SceneState) -> serde_json::Map<String, Value> {
    const LIMIT: usize = 64;
    let mut placed = Vec::new();
    let mut moved = Vec::new();
    let mut changed = Vec::new();
    for (id, o) in &after.objects {
        match before.objects.get(id) {
            None => placed.push(placed_json(o)),
            Some(b) if Arc::ptr_eq(b, o) => {}
            Some(b) => {
                let mut c = if b.xform != o.xform {
                    json!({
                        "name": o.name.as_str(),
                        "from": { "position": v3(b.position()), "rotation": v3(b.rotation()), "scale": v3(b.scale()) },
                        "to": { "position": v3(o.position()), "rotation": v3(o.rotation()), "scale": v3(o.scale()) },
                    })
                } else {
                    json!({ "name": o.name.as_str() })
                };
                if let (Some(a), Value::Object(m)) = (attached(o).filter(|a| attached(b) != Some(*a)), &mut c)
                {
                    m.insert("attached".into(), json!(a));
                }
                if b.xform != o.xform { moved.push(c) } else { changed.push(c) }
            }
        }
    }
    let removed: Vec<Value> = before
        .objects
        .iter()
        .filter(|(id, _)| !after.objects.contains_key(*id))
        .map(|(_, o)| json!(o.name.as_str()))
        .collect();
    let picked = names(after, after.selected_in_order().into_iter().filter(|id| !before.is_selected(*id)));
    let dropped = names(before, before.selected_in_order().into_iter().filter(|id| !after.is_selected(*id)));
    let selection = after.selected_in_order();
    let mut out = serde_json::Map::new();
    out.insert("picked".into(), Value::Array(picked.into_iter().take(LIMIT).collect()));
    out.insert("deselected".into(), Value::Array(dropped.into_iter().take(LIMIT).collect()));
    out.insert("selected".into(), json!(selection.len()));
    out.insert("selection".into(), Value::Array(names(after, selection.into_iter().take(LIMIT))));
    out.insert("placed".into(), Value::Array(placed.into_iter().take(LIMIT).collect()));
    out.insert("moved".into(), Value::Array(moved.into_iter().take(LIMIT).collect()));
    out.insert("changed".into(), Value::Array(changed.into_iter().take(LIMIT).collect()));
    out.insert("removed".into(), Value::Array(removed.into_iter().take(LIMIT).collect()));
    out.insert("mode".into(), json!(after.tools.mode.word()));
    // the bulk tools' own data
    let ai = |s: &SceneState| {
        s.bulk
            .ai_map
            .as_ref()
            .map_or((0, 0), |a| (a.node_count(), gz_scene::bulk::cmd::selected_nodes(s).len()))
    };
    let (ai_b, ai_a) = (ai(before), ai(after));
    if ai_b != ai_a {
        out.insert(
            "ai_nodes".into(),
            json!({ "before": ai_b.0, "after": ai_a.0, "selected_before": ai_b.1, "selected": ai_a.1 }),
        );
    }
    let marks = |s: &SceneState| {
        s.bulk
            .wallmarks
            .as_ref()
            .map_or((0, 0), |w| (w.count, w.model.as_ref().map_or(0, |m| m.selected_count())))
    };
    let (w_b, w_a) = (marks(before), marks(after));
    if w_b != w_a {
        out.insert(
            "wallmarks".into(),
            json!({ "before": w_b.0, "after": w_a.0, "selected_before": w_b.1, "selected": w_a.1 }),
        );
    }
    out
}

/// Where the gizmo stands on the pane now: its pivot and a point on each axis, in points.
fn gizmo_json(vp: &Viewport) -> Value {
    let (kind, action, anchor, cam, size, ppp) =
        vp.read(|s| (s.tool.gizmo, s.tool.action, s.anchor, s.nav.camera(), s.size, s.ppp.max(0.01)));
    let Some(anchor) = anchor.filter(|_| action == Action::Select && kind != GizmoType::None) else {
        return Value::Null;
    };
    let Some(at) = gizmo::screen_position(&cam, size, anchor.position) else { return Value::Null };
    let vp_m = cam.view_proj(size);
    let pt = |w: Vec3| {
        gz_render::project(&vp_m, size, w).map(|q| {
            let r = |x: f32| (f64::from(x / ppp) * 10.0).round() / 10.0;
            json!([r(q.x), r(q.y)])
        })
    };
    // the shafts' middle: on the arrow, clear of the pivot's square
    json!({
        "tool": match kind { GizmoType::Move => "move", GizmoType::Rotate => "rotate", GizmoType::Scale => "scale", GizmoType::None => "none" },
        "pivot": pt(at),
        "x": pt(at + Vec3::X * 0.55),
        "y": pt(at + Vec3::Y * 0.55),
        "z": pt(at + Vec3::Z * 0.55),
    })
}

/// `viewport_gesture kind= at= [to=] [button=] [mods=] [steps=]`.
pub(crate) async fn viewport_gesture(ed: Arc<Editor>, a: Args) -> Reply {
    let kind = a.str("kind").unwrap_or("click").to_ascii_lowercase();
    if !matches!(kind.as_str(), "click" | "double_click" | "drag" | "hover") {
        return Err(CmdError::invalid("kind must be click, double_click, drag or hover"));
    }
    let at = point(&a, "at")?.ok_or_else(|| CmdError::invalid("missing 'at' argument"))?;
    let to = point(&a, "to")?;
    if kind == "drag" && to.is_none() {
        return Err(CmdError::invalid("a drag needs 'to'"));
    }
    let to = to.unwrap_or(at);
    let button = a.str("button").unwrap_or("left").to_ascii_lowercase();
    if !matches!(button.as_str(), "left" | "right" | "middle") {
        return Err(CmdError::invalid("button must be left, right or middle"));
    }
    let mods = mods_of(&a)?;
    let moves = a.i64("steps").unwrap_or(8).clamp(1, 120) as usize;

    // nothing reaches the viewport while a dialog is up or another tab is drawn
    let refusal = gz_app::on_ui(&ed, |shell, ctx| {
        Ok(if shell.active() != gz_app::tabs::LEVEL {
            Some(("not_level", format!("the Level tab is not the active one ('{}' is)", shell.active())))
        } else if let Some(d) = gz_ui::dialog::modal_on_top(ctx) {
            Some(("modal", format!("a dialog is up: {}", d.title)))
        } else if shell.palette_open() {
            Some(("modal", "the command palette is up".to_owned()))
        } else {
            None
        })
    })
    .await?;
    if let Some((code, why)) = refusal {
        return Err(CmdError::new(code, why));
    }
    let vp = Viewport::of(&ed);
    if vp.read(|s| s.pane_pt[0] < 1.0) {
        // a tab that came up a moment ago: the pane lays itself out in the next frames
        ed.frames.after(2).await?;
    }
    let [w, h] = vp.read(|s| s.pane_pt);
    if w < 1.0 || h < 1.0 {
        return Err(CmdError::new("no_frame", "the viewport was not drawn yet"));
    }
    for p in [at, to] {
        if !(0.0..=w).contains(&p.x) || !(0.0..=h).contains(&p.y) {
            return Err(CmdError::invalid(format!(
                "({}, {}) is outside the viewport ({w:.0} x {h:.0} points)",
                p.x, p.y
            )));
        }
    }

    let svc = SceneService::of(&ed).ok();
    let before = svc.as_ref().map(|s| s.store.snapshot());
    let cam_before = vp.read(|s| (s.nav.position, s.nav.hpb));
    let log_from = ed.log.tail(1).first().map_or(0, |r| r.seq + 1);
    let frame0 = ed.frames.current().frame;
    let steps = steps_of(&kind, at, to, &button, moves);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    {
        let mut g = vp.gesture.lock();
        if g.is_some() {
            return Err(CmdError::busy("another gesture is still being fed to the viewport"));
        }
        *g = Some(Pending { steps, mods, done: Some(tx) });
    }
    // the pane takes a step a frame and asks for the next itself; a ping makes the first
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        ed.exec.wake().ping();
        match tokio::time::timeout(Duration::from_millis(25), &mut rx).await {
            Ok(_) => break,
            Err(_) if tokio::time::Instant::now() < deadline => {}
            Err(_) => {
                vp.gesture.lock().take();
                return Err(CmdError::new(
                    "no_frame",
                    "the viewport did not take the gesture: it was not drawn",
                ));
            }
        }
    }
    // what the steps started: the picks, the placements and the tools' edits
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while vp.tracked.load(Ordering::Acquire) > 0 {
        if tokio::time::Instant::now() > deadline {
            return Err(CmdError::new("busy", "the gesture's work did not end within 30 s"));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if let Some(s) = &svc {
        s.store.flush().await;
    }
    // the frames that show it: a pass of the scene as the gesture left it (the gizmo's anchor is that pass's)
    let mut stamp = ed.frames.after(2).await?;
    if let Some(s) = &svc {
        let rev = s.store.rev();
        for _ in 0..120 {
            if vp.drawn_scene(rev) {
                break;
            }
            stamp = ed.frames.after(1).await?;
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("kind".into(), json!(kind));
    out.insert("at".into(), json!([at.x, at.y]));
    if kind == "drag" {
        out.insert("to".into(), json!([to.x, to.y]));
    }
    out.insert("frames".into(), json!(stamp.frame.saturating_sub(frame0)));
    let (tool, view) = vp.read(|s| {
        (
            match (s.tool.action, s.tool.gizmo) {
                (Action::Place, _) => "place",
                (_, GizmoType::Move) => "move",
                (_, GizmoType::Rotate) => "rotate",
                (_, GizmoType::Scale) => "scale",
                _ => "select",
            },
            (s.nav.position, s.nav.hpb),
        )
    });
    out.insert("tool".into(), json!(tool));
    if let (Some(b), Some(s)) = (&before, &svc) {
        let after = s.store.snapshot();
        out.extend(scene_diff(&b.state, &after.state));
    }
    out.insert("camera_moved".into(), json!(view != cam_before));
    out.insert("gizmo".into(), gizmo_json(&vp));
    out.insert("viewport".into(), json!([w, h]));
    let lines: Vec<Value> = ed.log.since(log_from).iter().take(20).map(|r| json!(r.text)).collect();
    out.insert("log".into(), Value::Array(lines));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_arrives_presses_and_lets_go_at_one_point() {
        let s = steps_of("click", pos2(10.0, 20.0), pos2(10.0, 20.0), "left", 8);
        let kinds: Vec<(bool, bool, bool)> = s.iter().map(|x| (x.left, x.pressed, x.released)).collect();
        assert_eq!(
            kinds,
            [(false, false, false), (true, true, false), (false, false, true), (false, false, false)]
        );
        assert!(s.iter().all(|x| x.at == pos2(10.0, 20.0)));
    }

    #[test]
    fn a_drag_moves_in_even_steps_and_ends_where_it_was_sent() {
        let s = steps_of("drag", pos2(0.0, 0.0), pos2(80.0, 40.0), "right", 4);
        assert_eq!(s.len(), 1 + 1 + 4 + 1 + 1);
        let held: Vec<Pos2> = s.iter().filter(|x| x.right && !x.pressed).map(|x| x.at).collect();
        assert_eq!(held, [pos2(20.0, 10.0), pos2(40.0, 20.0), pos2(60.0, 30.0), pos2(80.0, 40.0)]);
        assert!(s.iter().all(|x| !x.left));
        assert_eq!(s.back().map(|x| x.at), Some(pos2(80.0, 40.0)));
    }

    #[test]
    fn a_double_click_is_two_presses() {
        let s = steps_of("double_click", pos2(1.0, 1.0), pos2(1.0, 1.0), "left", 8);
        assert_eq!(s.iter().filter(|x| x.pressed).count(), 2);
        assert_eq!(s.iter().filter(|x| x.released).count(), 2);
    }

    #[test]
    fn points_and_modifiers_parse_forgivingly() {
        let mut a = Args::new();
        a.insert("at".into(), json!([3, 4.5]));
        a.insert("to".into(), json!("10, 12"));
        a.insert("mods".into(), json!("ctrl+Shift"));
        assert_eq!(point(&a, "at").unwrap(), Some(pos2(3.0, 4.5)));
        assert_eq!(point(&a, "to").unwrap(), Some(pos2(10.0, 12.0)));
        assert_eq!(mods_of(&a).unwrap(), Mods { ctrl: true, shift: true, alt: false });
        a.insert("at".into(), json!({ "x": 1 }));
        assert!(point(&a, "at").is_err());
        a.insert("mods".into(), json!(["meta"]));
        assert!(mods_of(&a).is_err());
    }
}
