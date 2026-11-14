//! `viewport.gesture`: a gesture of the mouse - a click, a double click, a drag, a hover, a drop
//! of a drag payload - fed to the viewport. It enters where the window's input enters the pane
//! (`ViewportPane::input`), one step a frame, so the camera, the gizmo, the modes' presses,
//! strokes and marquees and the drop target take it exactly as they take the mouse; nothing is
//! sent to the operating system and no code path of its own decides what it does.
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
use gz_editor::{CmdError, Editor, Origin, Reply, Value, json};
use gz_scene::{ObjData, Object, SceneService, SceneState};
use gz_ui::drop::{Asset, Payload};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::gizmo::{self, GizmoType};
use crate::service::Viewport;

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
    mods: Mods,
    /// A drop: what the pointer carries, let go at the release.
    payload: Option<Arc<Payload>>,
    done: Option<tokio::sync::oneshot::Sender<()>>,
}

/// The step of a gesture the pane takes this frame.
pub(crate) struct Fed {
    pub step: Step,
    pub mods: Mods,
    pub last: bool,
    pub payload: Option<Arc<Payload>>,
}

impl Viewport {
    /// The step the pane takes this frame.
    pub(crate) fn gesture_step(&self) -> Option<Fed> {
        let mut g = self.gesture.lock();
        let p = g.as_mut()?;
        let step = p.steps.pop_front()?;
        let (mods, payload) = (p.mods, p.payload.clone());
        let last = p.steps.is_empty();
        if last && let Some(done) = g.take().and_then(|p| p.done) {
            let _ = done.send(());
        }
        Some(Fed { step, mods, last, payload })
    }
}

/// Starts a task of a gesture: `viewport.gesture` answers only once it has ended.
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

fn mods_of(names: &[String]) -> Result<Mods, CmdError> {
    let mut m = Mods::default();
    for n in names.iter().flat_map(|n| n.split(['+', ' '])).map(|n| n.trim().to_ascii_lowercase()) {
        match n.as_str() {
            "" => {}
            "ctrl" | "control" => m.ctrl = true,
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            other => return Err(CmdError::invalid(format!("unknown key '{other}' (ctrl, shift, alt)"))),
        }
    }
    Ok(m)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum GestureKind {
    /// A press and a release at one point.
    #[default]
    Click,
    DoubleClick,
    /// A press at `at`, the pointer to `to`, a release there.
    Drag,
    /// The pointer over `at`: answers where the gizmo's handles are.
    Hover,
    /// What `payload` or `assets` name, carried over `at` and let go there.
    Drop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

/// A Content Browser entry as the browser drags it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssetEntry {
    /// Its project-relative path, or a spawn section's name.
    path: String,
    /// What it is: object, group, particles, sound, spawn_section, ...
    #[serde(default)]
    kind: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GestureParams {
    /// What the mouse does; left out: a click.
    #[serde(default)]
    kind: GestureKind,
    /// Where, in points from the viewport's top left.
    at: [f32; 2],
    /// A drag: where it ends.
    #[serde(default)]
    to: Option<[f32; 2]>,
    /// The button; left out: left.
    #[serde(default)]
    button: Button,
    /// Keys held through it: ctrl, shift, alt.
    #[serde(default)]
    mods: Option<Vec<String>>,
    /// A drag: the pointer's moves between `at` and `to`; a drop: the frames it is carried over
    /// `at` before it is let go (8).
    #[serde(default)]
    steps: Option<u32>,
    /// A drop: what a row of the Place panel carries - `<kind>` or `<kind>:<reference>`
    /// (`object:props\box_wood_01`, `spawn:stalker`, `light`).
    #[serde(default)]
    payload: Option<Vec<String>>,
    /// A drop: what the Content Browser carries - its entries by path and kind.
    #[serde(default)]
    assets: Option<Vec<AssetEntry>>,
}

/// The frames of a gesture.
fn steps_of(kind: GestureKind, at: Pos2, to: Pos2, button: Button, moves: usize) -> VecDeque<Step> {
    let down = |pressed: bool, at: Pos2| Step {
        at,
        left: button == Button::Left,
        right: button == Button::Right,
        middle: button == Button::Middle,
        pressed,
        released: false,
    };
    let up = |at: Pos2| Step { at, released: true, ..Step::default() };
    let rest = |at: Pos2| Step { at, ..Step::default() };
    let mut s = VecDeque::new();
    // the pointer arrives first: the gizmo's handle under it is found before the press, as for
    // the mouse
    s.push_back(rest(at));
    match kind {
        GestureKind::Hover => {}
        // the pointer carries the payload over the spot a few frames, then lets it go there
        GestureKind::Drop => {
            for _ in 0..moves {
                s.push_back(rest(at));
            }
            s.push_back(up(at));
        }
        GestureKind::Click => {
            s.push_back(down(true, at));
            s.push_back(up(at));
        }
        GestureKind::DoubleClick => {
            for _ in 0..2 {
                s.push_back(down(true, at));
                s.push_back(up(at));
            }
        }
        GestureKind::Drag => {
            s.push_back(down(true, at));
            for i in 1..=moves {
                let t = i as f32 / moves as f32;
                s.push_back(down(false, at + (to - at) * t));
            }
            s.push_back(up(to));
        }
    }
    s.push_back(rest(if kind == GestureKind::Drag { to } else { at }));
    s
}

fn v3(v: Vec3) -> Value {
    let r = |x: f32| (f64::from(x) * 1000.0).round() / 1000.0;
    json!([r(v.x), r(v.y), r(v.z)])
}

/// Degrees, as every command speaks of a rotation.
fn deg(v: Vec3) -> Value {
    v3(Vec3::new(v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees()))
}

fn placed_json(o: &Object) -> Value {
    let mut m = json!({
        "name": o.name.as_str(),
        "class": o.class.name(),
        "position": v3(o.position()),
        "rotation": deg(o.rotation()),
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

/// The points of a way path that moved, by index.
fn points_moved(b: &Object, o: &Object) -> Option<Value> {
    let (ObjData::Way(wb), ObjData::Way(wa)) = (&b.data, &o.data) else { return None };
    let moved: Vec<Value> = wa
        .points
        .iter()
        .zip(&wb.points)
        .enumerate()
        .filter(|(_, (a, b))| a.position != b.position)
        .map(|(i, (a, b))| json!({ "point": i, "from": v3(b.position), "to": v3(a.position) }))
        .collect();
    (!moved.is_empty()).then(|| Value::Array(moved))
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
                        "from": { "position": v3(b.position()), "rotation": deg(b.rotation()), "scale": v3(b.scale()) },
                        "to": { "position": v3(o.position()), "rotation": deg(o.rotation()), "scale": v3(o.scale()) },
                    })
                } else {
                    json!({ "name": o.name.as_str() })
                };
                if let Value::Object(m) = &mut c {
                    if let Some(a) = attached(o).filter(|a| attached(b) != Some(*a)) {
                        m.insert("attached".into(), json!(a));
                    }
                    if let Some(p) = points_moved(b, o) {
                        m.insert("points".into(), p);
                    }
                }
                if b.xform != o.xform || c.get("points").is_some() { moved.push(c) } else { changed.push(c) }
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
    // the painted layers' own data
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

/// Where the gizmo's handles are on the pane now, in points: its centre, a point on each axis
/// handle and ring, and the middle of each plane handle.
pub(crate) fn gizmo_json(editor: &Editor, vp: &Viewport) -> Value {
    let (kind, anchor, cam, size, ppp, space) =
        vp.read(|s| (s.gizmo, s.anchor, s.nav.camera(), s.size, s.ppp.max(0.01), s.prefs.space));
    let Some(anchor) = anchor.filter(|_| kind != GizmoType::None) else { return Value::Null };
    let Some(at) = gizmo::screen_position(&cam, size, anchor.position) else { return Value::Null };
    let aimap = crate::pane::scene_mode(editor) == Some(gz_scene::SceneMode::AiMap);
    let axes = crate::pane::gizmo_axes(kind, &anchor, space);
    let vp_m = cam.view_proj(size);
    let proj = |w: Vec3| gz_render::project(&vp_m, size, w).map(|q| glam::Vec2::new(q.x / ppp, q.y / ppp));
    let Some(shapes) = gizmo::shapes(kind, at, axes, cam.forward, &proj, aimap) else { return Value::Null };
    let r = |x: f32| (f64::from(x) * 10.0).round() / 10.0;
    let pt = |p: glam::Vec2| json!([r(p.x), r(p.y)]);
    let mut out = serde_json::Map::new();
    out.insert("tool".into(), json!(kind.key()));
    out.insert("space".into(), json!(space.key()));
    out.insert("pivot".into(), pt(shapes.centre));
    for (st, a, b, _) in &shapes.axes {
        out.insert(st.key().into(), pt((*a + *b) * 0.5));
    }
    for (st, q) in &shapes.planes {
        out.insert(st.key().into(), pt((q[0] + q[1] + q[2] + q[3]) * 0.25));
    }
    for (st, ring) in &shapes.rings {
        // the ring's point lowest on the screen: on the ring, clear of the others' crossings
        if let Some(p) = ring.iter().copied().max_by(|a, b| a.y.total_cmp(&b.y)) {
            out.insert(st.key().into(), pt(p));
        }
    }
    if let Some(radius) = shapes.screen_ring {
        out.insert("screen".into(), pt(shapes.centre + glam::Vec2::new(radius, 0.0)));
    }
    Value::Object(out)
}

/// `viewport.gesture`.
pub(crate) async fn gesture(ed: Arc<Editor>, p: GestureParams, _o: Origin) -> Reply {
    let at = pos2(p.at[0], p.at[1]);
    let to = p.to.map(|t| pos2(t[0], t[1]));
    if p.kind == GestureKind::Drag && to.is_none() {
        return Err(CmdError::invalid("a drag needs 'to'"));
    }
    let to = to.unwrap_or(at);
    let mods = mods_of(p.mods.as_deref().unwrap_or_default())?;
    let moves = p.steps.unwrap_or(8).clamp(1, 120) as usize;
    let payload = if p.kind == GestureKind::Drop {
        let mut list: Vec<crate::place::Placeable> = Vec::new();
        for key in p.payload.as_deref().unwrap_or_default() {
            list.push(crate::place::Placeable::parse(key).ok_or_else(|| {
                CmdError::invalid(format!(
                    "'{key}' is not <kind> or <kind>:<reference> of a thing scene.place places"
                ))
            })?);
        }
        let payload = match (&p.assets, list.as_slice()) {
            (Some(assets), _) => Payload::Assets(
                assets
                    .iter()
                    .map(|a| Asset { path: a.path.clone(), kind: a.kind.clone(), file: None })
                    .collect(),
            ),
            (None, [one]) => one.payload(),
            (None, []) => return Err(CmdError::invalid("a drop needs 'payload' or 'assets'")),
            (None, _) => return Err(CmdError::invalid("a drop carries one payload key")),
        };
        if crate::place::Placeable::of_payload(&payload).is_empty() {
            return Err(CmdError::invalid("the viewport places none of what the drop carries"));
        }
        Some(Arc::new(payload))
    } else {
        None
    };

    // nothing reaches the viewport while a dialog is up or another tab is drawn
    let refusal = gz_app::on_ui(&ed, |shell, ctx| {
        Ok(if shell.active() != gz_app::tabs::LEVEL {
            Some(("refused", format!("the Level tab is not the active one ('{}' is)", shell.active())))
        } else if let Some(d) = gz_ui::dialog::modal_on_top(ctx) {
            Some(("refused", format!("a dialog is up: {}", d.title)))
        } else if shell.palette_open() {
            Some(("refused", "the command palette is up".to_owned()))
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
        return Err(CmdError::new("no_window", "the viewport was not drawn yet"));
    }
    for q in [at, to] {
        if !(0.0..=w).contains(&q.x) || !(0.0..=h).contains(&q.y) {
            return Err(CmdError::invalid(format!(
                "({}, {}) is outside the viewport ({w:.0} x {h:.0} points)",
                q.x, q.y
            )));
        }
    }

    let svc = SceneService::of(&ed).ok();
    let before = svc.as_ref().map(|s| s.store.snapshot());
    let cam_before = vp.read(|s| (s.nav.position, s.nav.hpb));
    let log_from = ed.log.tail(1).first().map_or(0, |r| r.seq + 1);
    let frame0 = ed.frames.current().frame;
    let steps = steps_of(p.kind, at, to, p.button, moves);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    {
        let mut g = vp.gesture.lock();
        if g.is_some() {
            return Err(CmdError::busy("another gesture is still being fed to the viewport"));
        }
        *g = Some(Pending { steps, mods, payload, done: Some(tx) });
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
                    "no_window",
                    "the viewport did not take the gesture: it was not drawn",
                ));
            }
        }
    }
    // what the steps started: the picks, the placements and the modes' edits
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while vp.tracked.load(Ordering::Acquire) > 0 {
        if tokio::time::Instant::now() > deadline {
            return Err(CmdError::new("failed", "the gesture's work did not end within 30 s"));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if let Some(s) = &svc {
        s.store.flush().await;
    }
    // the frames that show it: a pass of the scene as the gesture left it (the gizmo's anchor is
    // that pass's)
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
    out.insert("kind".into(), json!(p.kind));
    out.insert("at".into(), json!([at.x, at.y]));
    if p.kind == GestureKind::Drag {
        out.insert("to".into(), json!([to.x, to.y]));
    }
    out.insert("frames".into(), json!(stamp.frame.saturating_sub(frame0)));
    let (tool, view) = vp.read(|s| (s.gizmo.key(), (s.nav.position, s.nav.hpb)));
    out.insert("tool".into(), json!(tool));
    if let (Some(b), Some(s)) = (&before, &svc) {
        let after = s.store.snapshot();
        out.extend(scene_diff(&b.state, &after.state));
    }
    out.insert("camera_moved".into(), json!(view != cam_before));
    out.insert("gizmo".into(), gizmo_json(&ed, &vp));
    out.insert("viewport".into(), json!([w, h]));
    let lines: Vec<Value> = ed.log.since(log_from).iter().take(20).map(|r| json!(r.text)).collect();
    out.insert("log".into(), Value::Array(lines));
    Ok(Value::Object(out))
}

impl serde::Serialize for GestureKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            GestureKind::Click => "click",
            GestureKind::DoubleClick => "double_click",
            GestureKind::Drag => "drag",
            GestureKind::Hover => "hover",
            GestureKind::Drop => "drop",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_arrives_presses_and_lets_go_at_one_point() {
        let s = steps_of(GestureKind::Click, pos2(10.0, 20.0), pos2(10.0, 20.0), Button::Left, 8);
        let kinds: Vec<(bool, bool, bool)> = s.iter().map(|x| (x.left, x.pressed, x.released)).collect();
        assert_eq!(
            kinds,
            [(false, false, false), (true, true, false), (false, false, true), (false, false, false)]
        );
        assert!(s.iter().all(|x| x.at == pos2(10.0, 20.0)));
    }

    #[test]
    fn a_drag_moves_in_even_steps_and_ends_where_it_was_sent() {
        let s = steps_of(GestureKind::Drag, pos2(0.0, 0.0), pos2(80.0, 40.0), Button::Right, 4);
        assert_eq!(s.len(), 1 + 1 + 4 + 1 + 1);
        let held: Vec<Pos2> = s.iter().filter(|x| x.right && !x.pressed).map(|x| x.at).collect();
        assert_eq!(held, [pos2(20.0, 10.0), pos2(40.0, 20.0), pos2(60.0, 30.0), pos2(80.0, 40.0)]);
        assert!(s.iter().all(|x| !x.left));
        assert_eq!(s.back().map(|x| x.at), Some(pos2(80.0, 40.0)));
    }

    #[test]
    fn a_drop_carries_over_the_spot_and_lets_go_with_no_button_down() {
        let s = steps_of(GestureKind::Drop, pos2(5.0, 5.0), pos2(5.0, 5.0), Button::Left, 3);
        assert_eq!(s.len(), 1 + 3 + 1 + 1);
        assert_eq!(s.iter().filter(|x| x.released).count(), 1);
        assert!(s.iter().all(|x| !x.left && !x.pressed));
    }

    #[test]
    fn keys_parse_forgivingly() {
        assert_eq!(
            mods_of(&["ctrl+Shift".to_owned()]).ok(),
            Some(Mods { ctrl: true, shift: true, alt: false })
        );
        assert!(mods_of(&["meta".to_owned()]).is_err());
    }
}
