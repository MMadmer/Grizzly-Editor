//! The viewport's gestures of the AI Map, Detail Objects and Wallmark tools
//! (`TUI_ControlAIMapNodeAdd`, `TUI_ControlAIMapNodeSelect`, `TUI_ControlDOToolsSelect`,
//! `TUI_ControlWallmarkAdd` and the tools' `RaySelect` / `FrustumSelect`). Each sends the command
//! function `aimap_tool`, `details_tool` or `wallmark_tool` calls for the same action; a stroke
//! or a drag is one merged undo step sealed on release.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::{Vec2 as G2, Vec3};
use gz_editor::{Editor, Origin, SmolStr};
use gz_render::Ray;
use gz_scene::apply::SelectMode;
use gz_scene::bulk::cmd as bulk;
use gz_scene::bulk::{ViewAxes, ViewInfo};
use gz_scene::{ObjClass, SceneService};

use crate::camera::Buttons;
use crate::service::{Action, Viewport};

/// Whether a class is one of the bulk tools.
#[must_use]
pub fn is_bulk(class: ObjClass) -> bool {
    matches!(class, ObjClass::AiMap | ObjClass::DetailObjects | ObjClass::Wallmark)
}

fn svc(editor: &Arc<Editor>) -> Option<Arc<SceneService>> {
    SceneService::of(editor).ok().filter(|s| s.store.snapshot().valid)
}

fn report(editor: &Editor, r: Result<gz_editor::Value, gz_editor::CmdError>) {
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

/// A stroke of the AI map's brush: one undo step for the whole drag; one step at a time.
pub(crate) struct Paint {
    pub merge: SmolStr,
    busy: Arc<AtomicBool>,
}

/// What a press of Place with a bulk tool started.
pub(crate) enum BulkGesture {
    Done,
    Paint(Paint),
    /// Not a bulk tool.
    None,
}

/// `TUI_ControlAIMapNodeAdd::Start` / `TUI_ControlWallmarkAdd::Start`.
pub(crate) fn place_press(
    editor: &Arc<Editor>,
    vp: &Arc<Viewport>,
    ray: Ray,
    b: Buttons,
    serial: u64,
) -> BulkGesture {
    let Some(s) = svc(editor) else { return BulkGesture::None };
    match s.store.snapshot().tools.target {
        ObjClass::AiMap => {
            let paint = Paint {
                merge: SmolStr::new(format!("viewport.aimap.{serial}")),
                busy: Arc::new(AtomicBool::new(false)),
            };
            ai_step(editor, vp, &paint, ray, true, b.alt);
            // with Alt the press adds once and the tool goes back to Select
            if b.alt { BulkGesture::Done } else { BulkGesture::Paint(paint) }
        }
        ObjClass::Wallmark => {
            mark_place(editor, vp, ray, b);
            BulkGesture::Done
        }
        ObjClass::DetailObjects => BulkGesture::Done,
        _ => BulkGesture::None,
    }
}

/// One step of a brush stroke: `AddNode` where the ray meets the snap objects.
pub(crate) fn ai_step(
    editor: &Arc<Editor>,
    vp: &Arc<Viewport>,
    p: &Paint,
    ray: Ray,
    first: bool,
    back_to_select: bool,
) {
    if p.busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(s) = svc(editor) else {
        p.busy.store(false, Ordering::Release);
        return;
    };
    let far = vp.read(|v| v.nav.far);
    let (ed, vp, busy, merge) = (editor.clone(), vp.clone(), p.busy.clone(), p.merge.clone());
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
            None if first
                && s.store
                    .snapshot()
                    .bulk
                    .ai_map
                    .as_ref()
                    .is_none_or(|a| a.header.snap_objects.is_empty()) =>
            {
                ed.log.info("Fill object list and try again.");
            }
            None => {}
        }
        if back_to_select {
            s.store.seal();
            vp.with(|v| v.tool.action = Action::Select);
        }
        busy.store(false, Ordering::Release);
        ed.exec.wake().ping();
    });
}

/// The end of a stroke (`End`): one undo step, and back to Select unless Alt is held.
pub(crate) fn paint_end(editor: &Arc<Editor>, vp: &Viewport, b: Buttons) {
    if let Some(s) = svc(editor) {
        s.store.seal();
    }
    if !b.alt {
        vp.with(|v| v.tool.action = Action::Select);
    }
}

/// A wallmark laid where the mouse is; with Ctrl the selected one moves there. Without Alt the tool
/// goes back to Select.
fn mark_place(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Some(s) = svc(editor) else { return };
    let far = vp.read(|v| v.nav.far);
    let (ed, vp) = (editor.clone(), vp.clone());
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = if b.ctrl {
            bulk::mark_move(&ed, &s, origin, ray.origin, ray.dir, far).await
        } else {
            bulk::mark_add(&ed, &s, origin, ray.origin, ray.dir, far).await
        };
        let made = r.as_ref().is_ok_and(|v| {
            !v.get("added").is_some_and(gz_editor::Value::is_null)
                && v.get("moved") != Some(&gz_editor::Value::Bool(false))
        });
        report(&ed, r);
        if made && !b.alt {
            vp.with(|v| v.tool.action = Action::Select);
        }
        ed.exec.wake().ping();
    });
}

/// A click of the selection with a bulk tool current (`RaySelect` with Ctrl toggling): `false`
/// for another tool.
pub(crate) fn click(editor: &Arc<Editor>, vp: &Viewport, ray: Ray, b: Buttons, target: ObjClass) -> bool {
    if !is_bulk(target) {
        return false;
    }
    let Some(s) = svc(editor) else { return true };
    let far = vp.read(|v| v.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = match target {
            ObjClass::AiMap => bulk::ai_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await,
            ObjClass::DetailObjects => {
                bulk::slot_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await
            }
            _ => bulk::mark_pick(&ed, &s, origin, ray.origin, ray.dir, far, b.ctrl).await,
        };
        report(&ed, r);
        ed.exec.wake().ping();
    });
    true
}

/// The end of a marquee with a bulk tool current (`FrustumSelect`): `false` for another tool.
pub(crate) fn marquee(
    editor: &Arc<Editor>,
    vp: &Viewport,
    size: [u32; 2],
    a: G2,
    z: G2,
    target: ObjClass,
) -> bool {
    if !is_bulk(target) {
        return false;
    }
    let Some(s) = svc(editor) else { return true };
    let cam = vp.read(|v| v.nav.camera());
    let Some(planes) = crate::objtools::rect_planes(&cam, size, a, z) else { return true };
    let planes = planes.to_vec();
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = match target {
            ObjClass::AiMap => bulk::ai_select_frustum(&ed, &s, origin, planes, true).await,
            ObjClass::DetailObjects => {
                bulk::slots_select_frustum(&ed, &s, origin, planes, SelectMode::Add).await
            }
            _ => bulk::marks_select_frustum(&ed, &s, origin, planes, SelectMode::Add).await,
        };
        report(&ed, r);
        ed.exec.wake().ping();
    });
    true
}

/// The selected nodes a move drag holds, as at the press (`MoveStart`'s `SavePos`).
#[must_use]
pub fn held_nodes(editor: &Arc<Editor>) -> Vec<(u32, Vec3)> {
    svc(editor).map(|s| bulk::selected_nodes(&s.store.snapshot())).unwrap_or_default()
}

/// `MoveProcess`: the held nodes at their saved centres plus the drag.
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

/// `RotateProcess`: the selected nodes' planes turned about X or Z.
pub(crate) fn rotate_nodes(editor: &Arc<Editor>, about_x: bool, angle: f32, merge: &SmolStr, origin: Origin) {
    let Some(s) = svc(editor) else { return };
    let (ed, merge) = (editor.clone(), merge.clone());
    crate::gesture::spawn(editor, async move {
        report(&ed, bulk::ai_rotate(&ed, &s, origin, about_x, angle, Some(merge)).await);
        ed.exec.wake().ping();
    });
}

/// Delete, Select All, Deselect All, Invert Selection with a bulk tool current: `true` when it
/// took the key.
pub(crate) fn key(editor: &Arc<Editor>, which: &'static str) -> bool {
    let Some(s) = svc(editor) else { return false };
    if !is_bulk(s.store.snapshot().tools.target) {
        return false;
    }
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = if which == "delete" {
            bulk::remove_selected(&ed, origin).await
        } else {
            bulk::select_all(&ed, origin, which).await
        };
        if let Some(r) = r {
            report(&ed, r);
        }
        ed.exec.wake().ping();
    });
    true
}
