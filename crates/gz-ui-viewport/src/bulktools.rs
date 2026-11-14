//! The Paint modes' gestures in the viewport - the AI map's brush, node picking and the gizmo's
//! drags of nodes, the Foliage brush (paint, Shift erases), laying and picking decals. Each sends
//! the command function `scene.ai_map`, `scene.details` or `scene.wallmarks` runs for the same
//! action; a stroke or a drag is one undo step.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use glam::{Vec2 as G2, Vec3};
use gz_editor::{Editor, Origin, SmolStr, Value, json};
use gz_render::Ray;
use gz_scene::apply::SelectMode;
use gz_scene::bulk::cmd as bulk;
use gz_scene::bulk::{ViewAxes, ViewInfo};
use gz_scene::{ObjClass, SceneService};
use parking_lot::Mutex;

use crate::camera::Buttons;
use crate::service::Viewport;

fn svc(editor: &Arc<Editor>) -> Option<Arc<SceneService>> {
    SceneService::of(editor).ok().filter(|s| s.store.snapshot().valid)
}

fn report(editor: &Editor, r: Result<Value, gz_editor::CmdError>) {
    if let Err(e) = r {
        editor.log.error(e.message);
    }
}

/// The viewport's camera for the tools that lay things by it.
pub fn provide_axes(editor: &Arc<Editor>) {
    let axes = ViewAxes::of(editor);
    if axes.filled() {
        return;
    }
    let weak = Arc::downgrade(editor);
    axes.provide(move || {
        let ed = weak.upgrade()?;
        let vp = ed.services.get::<Viewport>()?;
        Some(vp.read(|s| {
            let cam = s.nav.camera();
            let right = cam.up.cross(cam.forward).normalize_or_zero();
            ViewInfo {
                position: cam.position,
                forward: cam.forward,
                top: cam.up,
                right,
                heading: s.nav.hpb.x,
            }
        }))
    });
}

/// The AI map's brush on the ground: its size in patches, as a radius in metres.
#[must_use]
pub fn ai_brush_radius(editor: &Arc<Editor>) -> f32 {
    svc(editor)
        .and_then(|s| {
            s.store.snapshot().bulk.ai_map.as_ref().map(|a| {
                let h = &a.header;
                (h.brush_size.max(1) as f32 * h.params.patch_size * 0.5).max(h.params.patch_size * 0.5)
            })
        })
        .unwrap_or(0.35)
}

/// A stroke of the AI map's brush: one undo step for the whole drag; one step at a time.
pub(crate) struct Paint {
    pub merge: SmolStr,
    busy: Arc<AtomicBool>,
}

/// A press of the AI map's brush: the stroke starts with nodes where it is.
pub(crate) fn ai_press(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, serial: u64) -> Paint {
    let paint = Paint {
        merge: SmolStr::new(format!("viewport.aimap.{serial}")),
        busy: Arc::new(AtomicBool::new(false)),
    };
    ai_step(editor, vp, &paint, ray, true);
    paint
}

/// One step of a brush stroke: nodes where the ray meets the AI map's ground.
pub(crate) fn ai_step(editor: &Arc<Editor>, vp: &Arc<Viewport>, p: &Paint, ray: Ray, first: bool) {
    if p.busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(s) = svc(editor) else {
        p.busy.store(false, Ordering::Release);
        return;
    };
    let far = vp.read(|v| v.nav.far);
    let (ed, busy, merge) = (editor.clone(), p.busy.clone(), p.merge.clone());
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let snap = s.store.snapshot();
        s.prefetch_library(&ed, &snap).await;
        let lib = s.library.clone();
        let snap = s.store.snapshot();
        let hit = ed.exec.cpu(move || bulk::ai_ground(&snap, &lib, ray.origin, ray.dir, far)).await;
        match hit {
            Some(at) => {
                let req = bulk::AddNodes {
                    positions: vec![at],
                    deselect: first,
                    brush: None,
                    ignore_constraints: None,
                    auto_link: None,
                    merge: Some(merge),
                };
                report(&ed, bulk::add_nodes(&ed, &s, origin, req).await);
            }
            None if first => ed.log.info("The brush met no ground the AI map lies on."),
            None => {}
        }
        busy.store(false, Ordering::Release);
        ed.exec.wake().ping();
    });
}

/// The end of an AI map stroke: one undo step.
pub(crate) fn paint_end(editor: &Arc<Editor>) {
    if let Some(s) = svc(editor) {
        s.store.seal();
    }
}

/// A spot of the Foliage brush: where the ray meets the ground, kept for the stroke.
pub(crate) fn foliage_spot(
    editor: &Arc<Editor>,
    spots: &Arc<Mutex<Vec<Vec3>>>,
    picking: &Arc<AtomicUsize>,
    ray: Ray,
) {
    let Some(s) = svc(editor) else { return };
    let far = Viewport::of(editor).read(|v| v.nav.far);
    let (ed, spots, picking) = (editor.clone(), spots.clone(), picking.clone());
    picking.fetch_add(1, Ordering::AcqRel);
    crate::gesture::spawn(editor, async move {
        if let Some((at, _)) = crate::pane::pick_ground(&ed, &s, ray, true, far).await {
            spots.lock().push(at);
        }
        picking.fetch_sub(1, Ordering::AcqRel);
    });
}

/// The end of a Foliage stroke: the brush's spots painted (or erased) as one `scene.details`
/// call, one undo step.
pub(crate) fn foliage_end(
    editor: &Arc<Editor>,
    vp: &Viewport,
    erase: bool,
    spots: Arc<Mutex<Vec<Vec3>>>,
    picking: Arc<AtomicUsize>,
) {
    let brush = vp.read(|s| s.brush.clone());
    if !erase && brush.mesh.is_none() {
        editor.log.info("Pick the detail mesh to paint in the Foliage panel first.");
        return;
    }
    let ed = editor.clone();
    crate::gesture::spawn(editor, async move {
        // the spots still being found
        for _ in 0..400 {
            if picking.load(Ordering::Acquire) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let at: Vec<[f32; 3]> = spots.lock().iter().map(|p| p.to_array()).collect();
        if at.is_empty() {
            ed.log.info("The brush met no ground to paint on.");
            return;
        }
        let mut a = gz_editor::Args::new();
        a.insert("action".into(), json!(if erase { "erase" } else { "paint" }));
        a.insert("positions".into(), json!(at));
        a.insert("radius".into(), json!(brush.radius));
        a.insert("density".into(), json!(brush.density));
        if let Some(m) = &brush.mesh {
            a.insert("object".into(), json!(m.as_str()));
        }
        let r = ed.call("scene.details", a, ed.ui_origin()).await;
        if r.get("ok").and_then(Value::as_bool) != Some(true) {
            ed.log.error(
                r.get("error").and_then(Value::as_str).unwrap_or("The details were not painted.").to_owned(),
            );
        }
        ed.exec.wake().ping();
    });
}

/// A decal laid where the pointer is; with `move_selected` the selected one moves there.
pub(crate) fn mark_place(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, move_selected: bool) {
    let Some(s) = svc(editor) else { return };
    let far = vp.read(|v| v.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = if move_selected {
            bulk::mark_move(&ed, &s, origin, ray.origin, ray.dir, far).await
        } else {
            bulk::mark_add(&ed, &s, origin, ray.origin, ray.dir, far).await
        };
        report(&ed, r);
        ed.exec.wake().ping();
    });
}

/// A Ctrl click of a Paint mode: the layer's own thing under the pointer picked (Ctrl toggles).
pub(crate) fn click(editor: &Arc<Editor>, vp: &Viewport, ray: Ray, b: Buttons, layer: ObjClass) {
    let Some(s) = svc(editor) else { return };
    let far = vp.read(|v| v.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = match layer {
            ObjClass::AiMap => bulk::ai_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await,
            ObjClass::DetailObjects => {
                bulk::slot_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await
            }
            _ => bulk::mark_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await,
        };
        report(&ed, r);
        ed.exec.wake().ping();
    });
}

/// The end of a Ctrl marquee of a Paint mode: the layer's own things in the box selected.
pub(crate) fn marquee(editor: &Arc<Editor>, vp: &Viewport, size: [u32; 2], a: G2, z: G2, layer: ObjClass) {
    let Some(s) = svc(editor) else { return };
    let cam = vp.read(|v| v.nav.camera());
    let Some(planes) = crate::objtools::rect_planes(&cam, size, a, z) else { return };
    let planes = planes.to_vec();
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = match layer {
            ObjClass::AiMap => bulk::ai_select_frustum(&ed, &s, origin, planes, true).await,
            ObjClass::DetailObjects => {
                bulk::slots_select_frustum(&ed, &s, origin, planes, SelectMode::Add).await
            }
            _ => bulk::marks_select_frustum(&ed, &s, origin, planes, SelectMode::Add).await,
        };
        report(&ed, r);
        ed.exec.wake().ping();
    });
}

/// The selected nodes a move drag holds, as at the press.
#[must_use]
pub fn held_nodes(editor: &Arc<Editor>) -> Vec<(u32, Vec3)> {
    svc(editor).map(|s| bulk::selected_nodes(&s.store.snapshot())).unwrap_or_default()
}

/// A move drag of nodes: each held node at its place at the press moved by `to`.
pub(crate) fn move_nodes(
    editor: &Arc<Editor>,
    held: &[(u32, Vec3)],
    to: impl Fn(Vec3) -> Vec3,
    merge: &SmolStr,
    origin: Origin,
) {
    let Some(s) = svc(editor) else { return };
    let moves: Vec<(u32, Vec3)> = held.iter().map(|(i, p)| (*i, to(*p))).collect();
    let (ed, merge) = (editor.clone(), merge.clone());
    crate::gesture::spawn(editor, async move {
        report(&ed, bulk::ai_move(&ed, &s, origin, moves, Some(merge)).await);
        ed.exec.wake().ping();
    });
}

/// A turn of the selected nodes' planes about X or Z, by `angle` more.
pub(crate) fn rotate_nodes(editor: &Arc<Editor>, about_x: bool, angle: f32, merge: &SmolStr, origin: Origin) {
    let Some(s) = svc(editor) else { return };
    let (ed, merge) = (editor.clone(), merge.clone());
    crate::gesture::spawn(editor, async move {
        report(&ed, bulk::ai_rotate(&ed, &s, origin, about_x, angle, Some(merge)).await);
        ed.exec.wake().ping();
    });
}
