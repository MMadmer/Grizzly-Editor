//! The viewport's gestures of the Sector, Way and Shape tools (`TUI_ControlSectorAdd`,
//! `TUI_ControlWayPointAdd`, `CWayObject::RaySelect` / `FrustumSelect` / `Move` in Points mode,
//! `TUI_ControlShapeAdd`). Each reads a snapshot, picks on the CPU pool and sends one of
//! `gz-scene`'s object tool edits - the same functions `sector_tool`, `way_tool` and `shape_tool`
//! call; a drag is one merged undo step sealed on release.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::{Vec2 as G2, Vec3, Vec4};
use gz_editor::{Editor, SmolStr};
use gz_render::Ray;
use gz_scene::apply::SelectMode;
use gz_scene::objtools::{self, Ctl, ToolEdit, cmd as tools_cmd, geom, sector, shape, way};
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneCmd, SceneService};

use crate::camera::Buttons;
use crate::service::{Action, Viewport};

/// Whether a click in the viewport adds now (Place), what the Sector tool's Edit section is for.
#[must_use]
pub fn placing(editor: &Editor) -> bool {
    Viewport::of(editor).read(|s| s.tool.action == Action::Place)
}

/// Switches the viewport between Place and Select.
pub fn set_placing(editor: &Editor, on: bool) {
    Viewport::of(editor).with(|s| {
        s.tool.action = if on { Action::Place } else { Action::Select };
        if on {
            s.tool.gizmo = crate::gizmo::GizmoType::None;
        }
    });
}

/// What a press of an object tool started.
pub(crate) enum ToolGesture {
    /// The press was the tool's whole gesture.
    Done,
    /// Meshes added to (or taken from) the selected sector as the mouse moves (`saAddMesh`,
    /// `saDelMesh`).
    Paint(Paint),
    /// A box drawn to pick meshes (`saMeshBoxSelection`).
    BoxPick,
    /// No tool of these took the press.
    None,
}

/// A paint of meshes: one undo step for the whole drag; one pick at a time.
pub(crate) struct Paint {
    pub add: bool,
    pub sector: ObjId,
    pub merge: SmolStr,
    busy: Arc<AtomicBool>,
}

fn svc(editor: &Arc<Editor>) -> Option<Arc<SceneService>> {
    SceneService::of(editor).ok().filter(|s| s.store.snapshot().valid)
}

/// A press of Place for the classes whose Place is a tool of its own. `serial` names the
/// gesture (the merge key of its undo step).
pub(crate) fn place_press(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons, serial: u64) -> ToolGesture {
    let Some(svc) = svc(editor) else { return ToolGesture::None };
    let snap = svc.store.snapshot();
    let t = snap.tools.obj;
    match snap.tools.target {
        ObjClass::Sector => {
            if t.sector.create_single {
                sector_new(editor, vp, ray, b, false);
                return ToolGesture::Done;
            }
            if t.sector.create_multiple {
                sector_new(editor, vp, ray, b, true);
                return ToolGesture::Done;
            }
            let sel = match sector::selected_sector(&snap) {
                Ok(s) => s,
                Err(e) => {
                    editor.log.error(e.message);
                    return ToolGesture::Done;
                }
            };
            let Some(sector) = sel else { return ToolGesture::Done };
            if t.sector.box_pick {
                return ToolGesture::BoxPick;
            }
            let paint = Paint {
                add: t.sector.mesh_add,
                sector,
                merge: SmolStr::new(format!("viewport.sector.{serial}")),
                busy: Arc::new(AtomicBool::new(false)),
            };
            paint_step(editor, vp, &paint, ray);
            ToolGesture::Paint(paint)
        }
        ObjClass::Way if t.way_points => {
            way_add_point(editor, vp, ray, b);
            ToolGesture::Done
        }
        ObjClass::Shape if t.attach_on_click => {
            shape_attach(editor, vp, ray, b);
            ToolGesture::Done
        }
        _ => ToolGesture::None,
    }
}

/// One step of a mesh paint: the mesh under the mouse added to or taken from the sector.
pub(crate) fn paint_step(editor: &Arc<Editor>, vp: &Viewport, p: &Paint, ray: Ray) {
    if p.busy.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(svc) = svc(editor) else {
        p.busy.store(false, Ordering::Release);
        return;
    };
    let far = vp.read(|s| s.nav.far);
    let (ed, busy, add, sector, merge) = (editor.clone(), p.busy.clone(), p.add, p.sector, p.merge.clone());
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        let hit = ed.exec.cpu(move || geom::pick_mesh(&snap, &lib, ray.origin, ray.dir, far)).await;
        if let Some(h) = hit {
            let pick = (h.object.id, h.mesh);
            let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
                let (mut b, v) = sector::edit_meshes(s, lib, sector, &[pick], add, false, if add { "Add Mesh" } else { "Remove Mesh" })?;
                b.merge = Some(merge.clone());
                Ok((b, v))
            })
            .await;
            match r {
                Ok(v) => {
                    if let Some(n) = v.get("note").and_then(|n| n.as_str()) {
                        ed.log.info(n.to_owned());
                    }
                }
                Err(e) => ed.log.error(e.message),
            }
        }
        busy.store(false, Ordering::Release);
        ed.exec.wake().ping();
    });
}

/// `AddSector` / `AddSectors`: the mesh (or every mesh of the object) under the mouse becomes a
/// sector; New Sector From Mesh goes off after one unless Alt is held.
fn sector_new(editor: &Arc<Editor>, vp: &Viewport, ray: Ray, b: Buttons, multiple: bool) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        let Some(h) = ed.exec.cpu(move || geom::pick_mesh(&snap, &lib, ray.origin, ray.dir, far)).await else { return };
        let (oid, mesh) = (h.object.id, h.mesh);
        let seed = tools_cmd::time_seed();
        let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
            if multiple { sector::new_from_object(s, lib, oid, seed) } else { sector::new_from_mesh(s, lib, oid, mesh) }
        })
        .await;
        match r {
            Ok(_) if !multiple && !b.alt => {
                let _ = objtools::set_state(&svc, origin, |st| st.sector.create_single = false).await;
            }
            Ok(_) => {}
            Err(e) => ed.log.error(e.message),
        }
        ed.exec.wake().ping();
    });
}

/// The frustum of a rectangle of the viewport (pixels): the camera's frustum squeezed to it.
#[must_use]
pub fn rect_planes(cam: &gz_render::Camera, size: [u32; 2], a: G2, b: G2) -> Option<[Vec4; 6]> {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let (x0, x1) = (a.x.min(b.x) / w * 2.0 - 1.0, a.x.max(b.x) / w * 2.0 - 1.0);
    let (y0, y1) = (1.0 - a.y.max(b.y) / h * 2.0, 1.0 - a.y.min(b.y) / h * 2.0);
    if (x1 - x0).abs() < 1e-6 || (y1 - y0).abs() < 1e-6 {
        return None;
    }
    let s = glam::Mat4::from_cols(
        Vec4::new(2.0 / (x1 - x0), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / (y1 - y0), 0.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 0.0),
        Vec4::new(-(x1 + x0) / (x1 - x0), -(y1 + y0) / (y1 - y0), 0.0, 1.0),
    );
    Some(gz_render::Frustum::from_view_proj(&(s * cam.view_proj(size))).planes)
}

/// The end of a box pick (`TUI_ControlSectorAdd::End`): the meshes in the box added to or taken
/// from the selected sector.
pub(crate) fn box_pick_end(editor: &Arc<Editor>, vp: &Viewport, size: [u32; 2], a: G2, b: G2) {
    let Some(svc) = svc(editor) else { return };
    let cam = vp.read(|s| s.nav.camera());
    let Some(planes) = rect_planes(&cam, size, a, b) else { return };
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
            let Some(id) = sector::selected_sector(s)? else {
                return Ok((objtools::Batch::new("Box Pick", s.serial), gz_editor::Value::Null));
            };
            let add = s.tools.obj.sector.mesh_add;
            let picks = sector::meshes_in_frustum(s, lib, &planes);
            sector::edit_meshes(s, lib, id, &picks, add, !add, if add { "Add Meshes" } else { "Remove Meshes" })
        })
        .await;
        match r {
            Ok(v) => {
                if let Some(n) = v.get("note").and_then(|n| n.as_str()) {
                    ed.log.info(n.to_owned());
                }
            }
            Err(e) => ed.log.error(e.message),
        }
        ed.exec.wake().ping();
    });
}

/// `TUI_ControlWayPointAdd::Start`: a point of the one selected way where the ground is hit,
/// linked from its selected point with Auto Link; Alt keeps adding.
fn way_add_point(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Some(svc) = svc(editor) else { return };
    let snap = svc.store.snapshot();
    let sel: Vec<ObjId> = snap.of_class(ObjClass::Way).filter(|o| snap.is_selected(o.id) && o.flags.visible).map(|o| o.id).collect();
    let [way_id] = sel.as_slice() else {
        editor.log.info("Select one WayObject.");
        return;
    };
    let way_id = *way_id;
    let (object_snap, grid, far) = vp.read(|s| {
        (s.settings.object_snap, s.snap_on(crate::gizmo::GizmoType::Move).then_some(s.step(crate::gizmo::GizmoType::Move)), s.nav.far)
    });
    let (ed, vp) = (editor.clone(), vp.clone());
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        if let Some((at, _)) = crate::pane::pick_ground(&ed, &svc, ray, object_snap, grid, far).await {
            let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, _, _| tools_cmd::add_point_batch(s, way_id, at)).await;
            if let Err(e) = r {
                ed.log.error(e.message);
            }
        }
        if !b.alt {
            vp.with(|s| s.tool.action = Action::Select);
        }
        ed.exec.wake().ping();
    });
}

/// Attach on Click: the shape under the mouse joins the one selected shape; the switch goes off
/// after one unless Alt is held.
fn shape_attach(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let (ed, vp) = (editor.clone(), vp.clone());
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let s = snap.clone();
        let hit = ed
            .exec
            .cpu(move || {
                pick.ray_pick(&s, &lib, ray.origin, ray.dir, far, &|o: &Object| {
                    o.class == ObjClass::Shape && o.flags.visible && s.tools.part(ObjClass::Shape).visible
                })
            })
            .await;
        let Some(hit) = hit else { return };
        let from = hit.id;
        let sel = objtools::selected_of(&snap, ObjClass::Shape);
        if sel.len() != 1 {
            ed.log.error("Select one shape.");
            return;
        }
        if sel.first() == Some(&from) {
            return;
        }
        match objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, _, _| shape::attach_to_selected(s, from)).await {
            Ok(_) if !b.alt => {
                let _ = objtools::set_state(&svc, origin, |st| st.attach_on_click = false).await;
                vp.with(|s| s.tool.action = Action::Select);
            }
            Ok(_) => {}
            Err(e) => ed.log.error(e.message),
        }
        ed.exec.wake().ping();
    });
}

/// What a click adds with the Shape tool: a sphere or a box, as the panel says.
#[must_use]
pub fn new_shape(editor: &Arc<Editor>) -> ObjData {
    let sphere = svc(editor).is_some_and(|s| s.store.snapshot().tools.obj.shape_sphere);
    ObjData::Shape(shape::new_shape_data(sphere))
}

/// Whether the Way tool works on points now (Points mode with the Way tool current).
#[must_use]
pub fn way_points_mode(editor: &Arc<Editor>) -> bool {
    svc(editor).is_some_and(|s| {
        let snap = s.store.snapshot();
        snap.tools.target == ObjClass::Way && snap.tools.obj.way_points
    })
}

/// A click of the selection in Points mode (`CWayObject::RaySelect`): without Ctl the points of
/// the selected ways are let go first; the nearest point of a visible way is selected (Ctrl
/// toggles it).
pub(crate) fn way_point_click(editor: &Arc<Editor>, vp: &Viewport, ray: Ray, b: Buttons) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let any = vp.read(|s| s.settings.pick_any_class);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        if !b.ctrl {
            if any {
                for c in ObjClass::ALL.iter().copied().filter(|c| *c != ObjClass::Way) {
                    let _ = svc.store.send(SceneCmd::SelectClass { class: Some(c), select: false }, origin).await;
                }
            }
            for id in objtools::selected_of(&snap, ObjClass::Way) {
                let _ = svc
                    .store
                    .send(SceneCmd::ObjTool(Box::new(ToolEdit::SelectPoints { way: id, points: Vec::new(), mode: SelectMode::Set })), origin)
                    .await;
            }
        }
        let snap = svc.store.snapshot();
        let mut best: Option<(ObjId, usize, f32)> = None;
        if snap.tools.part(ObjClass::Way).visible {
            for o in snap.of_class(ObjClass::Way).filter(|o| o.flags.visible) {
                let ObjData::Way(w) = &o.data else { continue };
                if let Some((i, d)) = way::pick_point(w, ray.origin, ray.dir, best.map_or(far, |x| x.2)) {
                    best = Some((o.id, i, d));
                }
            }
        }
        if let Some((id, i, _)) = best {
            let mode = if b.ctrl { SelectMode::Toggle } else { SelectMode::Add };
            let _ = svc.store.send(SceneCmd::ObjTool(Box::new(ToolEdit::SelectPoints { way: id, points: vec![i], mode })), origin).await;
        }
        ed.exec.wake().ping();
    });
}

/// A marquee in Points mode (`CWayObject::FrustumSelect`): the points of the selected ways inside
/// the box are selected.
pub(crate) fn way_points_marquee(editor: &Arc<Editor>, vp: &Viewport, size: [u32; 2], a: G2, b: G2) {
    let Some(svc) = svc(editor) else { return };
    let cam = vp.read(|s| s.nav.camera());
    let Some(planes) = rect_planes(&cam, size, a, b) else { return };
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        for id in objtools::selected_of(&snap, ObjClass::Way) {
            let Some(ObjData::Way(w)) = snap.get(id).map(|o| &o.data) else { continue };
            let r = gz_scene::objects::WAYPOINT_RADIUS;
            let inside: Vec<usize> = w
                .points
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    let c = p.position + Vec3::Y * r;
                    planes.iter().all(|pl| pl.truncate().dot(c) + pl.w >= -r)
                })
                .map(|(i, _)| i)
                .collect();
            if !inside.is_empty() {
                let _ = svc
                    .store
                    .send(SceneCmd::ObjTool(Box::new(ToolEdit::SelectPoints { way: id, points: inside, mode: SelectMode::Add })), origin)
                    .await;
            }
        }
        ed.exec.wake().ping();
    });
}

/// The selected points of the ways a move drag holds, as they are at the press.
#[must_use]
pub fn held_points(editor: &Arc<Editor>, ways: &[ObjId]) -> Vec<(ObjId, Vec<(usize, Vec3)>)> {
    let Some(svc) = svc(editor) else { return Vec::new() };
    let snap = svc.store.snapshot();
    ways.iter()
        .filter_map(|id| match snap.get(*id).map(|o| &o.data) {
            Some(ObjData::Way(w)) => Some((
                *id,
                w.points.iter().enumerate().filter(|(_, p)| p.selected).map(|(i, p)| (i, p.position)).collect(),
            )),
            _ => None,
        })
        .collect()
}

/// `CWayObject::Move` in Points mode: the held points moved by `delta`, one merged undo step.
pub(crate) fn move_points(svc: &SceneService, held: &[(ObjId, Vec<(usize, Vec3)>)], delta: Vec3, merge: &SmolStr, origin: gz_editor::Origin) {
    for (id, pts) in held {
        if pts.is_empty() {
            continue;
        }
        let points = pts.iter().map(|(i, p)| (*i, *p + delta)).collect();
        svc.store.post(SceneCmd::ObjTool(Box::new(ToolEdit::MovePoints { way: *id, points, merge: Some(merge.clone()) })), origin);
    }
}
