//! The viewport's gestures of the zones (cells made from meshes, meshes painted or boxed into a
//! cell), of Place mode's way paths (a click adds a point to the selected way) and attaching on a
//! click. Each reads a snapshot, picks on the CPU pool and sends one of `gz-scene`'s object tool
//! edits - the same functions `scene.zones`, `scene.way` and `scene.shape` call; a drag is one
//! merged undo step sealed on release.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::{Vec2 as G2, Vec3, Vec4};
use gz_editor::{Editor, SmolStr, Value};
use gz_render::Ray;
use gz_scene::objtools::{self, Ctl, ToolEdit, cmd as tools_cmd, geom, sector, shape};
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneCmd, SceneService};

use crate::camera::Buttons;
use crate::service::Viewport;

/// Whether a click in the viewport places now: the scene is in Place mode.
#[must_use]
pub fn placing(editor: &Editor) -> bool {
    crate::pane::scene_mode(editor) == Some(gz_scene::SceneMode::Place)
}

/// Puts Place mode up (with the kind it has armed) or Select back.
pub fn set_placing(editor: &Arc<Editor>, on: bool) {
    let mut a = gz_editor::Args::new();
    a.insert("mode".into(), Value::String(if on { "place" } else { "select" }.into()));
    editor.post("scene.mode", a);
}

/// What a press of an object tool started.
pub(crate) enum ToolGesture {
    /// The press was the tool's whole gesture.
    Done,
    /// Meshes added to (or taken from) the selected sector as the mouse moves.
    Paint(Paint),
    /// A box drawn to pick meshes.
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
pub(crate) fn place_press(
    editor: &Arc<Editor>,
    vp: &Arc<Viewport>,
    ray: Ray,
    b: Buttons,
    serial: u64,
) -> ToolGesture {
    let Some(svc) = svc(editor) else { return ToolGesture::None };
    let snap = svc.store.snapshot();
    let t = snap.tools.obj;
    match snap.tools.mode_kind() {
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
        // with a way path selected, Place adds points to it
        ObjClass::Way if !objtools::selected_of(&snap, ObjClass::Way).is_empty() => {
            way_add_point(editor, vp, ray);
            ToolGesture::Done
        }
        ObjClass::Shape if t.attach_on_click => {
            shape_attach(editor, vp, ray, b);
            ToolGesture::Done
        }
        ObjClass::SpawnPoint if t.spawn_attach => {
            spawn_attach(editor, vp, ray, b);
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
    crate::gesture::spawn(editor, async move {
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        let hit = ed.exec.cpu(move || geom::pick_mesh(&snap, &lib, ray.origin, ray.dir, far)).await;
        if let Some(h) = hit {
            let pick = (h.object.id, h.mesh);
            let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
                let (mut b, v) = sector::edit_meshes(
                    s,
                    lib,
                    sector,
                    &[pick],
                    add,
                    false,
                    if add { "Add Mesh" } else { "Remove Mesh" },
                )?;
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

/// New Cell From Mesh / From Object: the mesh (or every mesh of the object) under the mouse becomes a
/// sector; New Sector From Mesh goes off after one unless Alt is held.
fn sector_new(editor: &Arc<Editor>, vp: &Viewport, ray: Ray, b: Buttons, multiple: bool) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        let Some(h) = ed.exec.cpu(move || geom::pick_mesh(&snap, &lib, ray.origin, ray.dir, far)).await
        else {
            return;
        };
        let (oid, mesh) = (h.object.id, h.mesh);
        let seed = tools_cmd::time_seed();
        let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
            if multiple {
                sector::new_from_object(s, lib, oid, seed)
            } else {
                sector::new_from_mesh(s, lib, oid, mesh)
            }
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

/// The end of a box pick: the meshes in the box added to or taken
/// from the selected sector.
pub(crate) fn box_pick_end(editor: &Arc<Editor>, vp: &Viewport, size: [u32; 2], a: G2, b: G2) {
    let Some(svc) = svc(editor) else { return };
    let cam = vp.read(|s| s.nav.camera());
    let Some(planes) = rect_planes(&cam, size, a, b) else { return };
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, lib, _| {
            let Some(id) = sector::selected_sector(s)? else {
                return Ok((objtools::Batch::new("Box Pick", s.serial), gz_editor::Value::Null));
            };
            let add = s.tools.obj.sector.mesh_add;
            let picks = sector::meshes_in_frustum(s, lib, &planes);
            sector::edit_meshes(
                s,
                lib,
                id,
                &picks,
                add,
                !add,
                if add { "Add Meshes" } else { "Remove Meshes" },
            )
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

/// A point of the one selected way where the ground is hit, linked from its selected point with
/// Auto Link. Place mode stays up for the next one.
fn way_add_point(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray) {
    let Some(svc) = svc(editor) else { return };
    let snap = svc.store.snapshot();
    let sel: Vec<ObjId> = snap
        .of_class(ObjClass::Way)
        .filter(|o| snap.is_selected(o.id) && o.flags.visible)
        .map(|o| o.id)
        .collect();
    let [way_id] = sel.as_slice() else {
        editor.log.info("Select one WayObject.");
        return;
    };
    let way_id = *way_id;
    let _ = vp;
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        if let Some((at, _)) = crate::place::landing(&ed, &svc, ray).await {
            let r = objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, _, _| {
                tools_cmd::add_point_batch(s, way_id, at)
            })
            .await;
            if let Err(e) = r {
                ed.log.error(e.message);
            }
        }
        ed.exec.wake().ping();
    });
}

/// Attach on Click: the shape under the mouse joins the one selected shape; the switch goes off
/// after one unless Alt is held.
fn shape_attach(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
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
        match objtools::commit(&ed, &svc, origin, Ctl::default(), move |s, _, _| {
            shape::attach_to_selected(s, from)
        })
        .await
        {
            Ok(_) if !b.alt => {
                let _ = objtools::set_state(&svc, origin, |st| st.attach_on_click = false).await;
            }
            Ok(_) => {}
            Err(e) => ed.log.error(e.message),
        }
        ed.exec.wake().ping();
    });
}

/// Attach on Click: the object under the mouse
/// attaches to the one selected spawn point; the switch goes off after one unless Alt is held.
fn spawn_attach(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Some(svc) = svc(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    crate::gesture::spawn(editor, async move {
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let s = snap.clone();
        let hit = ed
            .exec
            .cpu(move || {
                pick.ray_pick(&s, &lib, ray.origin, ray.dir, far, &|o: &Object| {
                    o.class != ObjClass::SpawnPoint && o.flags.visible && s.tools.part(o.class).visible
                })
            })
            .await;
        let Some(name) = hit.and_then(|h| snap.get(h.id).map(|o| o.name.to_string())) else {
            ed.log.error("Attach impossible.");
            return;
        };
        let mut a = gz_editor::Args::new();
        a.insert("object".into(), Value::String(name));
        let r = ed.call("scene.attach", a, origin).await;
        if r.get("ok").and_then(Value::as_bool) != Some(true) {
            ed.log.error(r.get("error").and_then(Value::as_str).unwrap_or("Attach impossible.").to_owned());
        } else if !b.alt {
            let _ = objtools::set_state(&svc, origin, |st| st.spawn_attach = false).await;
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

/// The selected points of the ways a move drag holds, as they are at the press.
#[must_use]
pub fn held_points(editor: &Arc<Editor>, ways: &[ObjId]) -> Vec<(ObjId, Vec<(usize, Vec3)>)> {
    let Some(svc) = svc(editor) else { return Vec::new() };
    let snap = svc.store.snapshot();
    ways.iter()
        .filter_map(|id| match snap.get(*id).map(|o| &o.data) {
            Some(ObjData::Way(w)) => Some((
                *id,
                w.points
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.selected)
                    .map(|(i, p)| (i, p.position))
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// A drag in Points mode: the held points moved by `delta`, one merged undo step.
pub(crate) fn move_points(
    svc: &SceneService,
    held: &[(ObjId, Vec<(usize, Vec3)>)],
    delta: Vec3,
    merge: &SmolStr,
    origin: gz_editor::Origin,
) {
    for (id, pts) in held {
        if pts.is_empty() {
            continue;
        }
        let points = pts.iter().map(|(i, p)| (*i, *p + delta)).collect();
        svc.store.post(
            SceneCmd::ObjTool(Box::new(ToolEdit::MovePoints {
                way: *id,
                points,
                merge: Some(merge.clone()),
            })),
            origin,
        );
    }
}
