//! The viewport's commands: `viewport.camera`, `viewport.view`, `viewport.play`,
//! `viewport.gesture` (in `gesture.rs`), `shot.viewport` and `perf.frames`.

use std::sync::Arc;

use base64::Engine;
use glam::Vec3;
use gz_editor::{CmdError, Editor, Origin, Registry, Reply, Spec, Value, json};
use gz_scene::apply::SelectMode;
use gz_scene::{SceneCmd, SceneService};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::camera::Target;
use crate::gizmo::{self, GizmoType, Space};
use crate::lighting::{self, hour_text};
use crate::menus;
use crate::place::Placeable;
use crate::service::{self, ANGLE_STEPS, Brush, MOVE_STEPS, SCALE_STEPS, SHOW_FLAGS, ViewMode, Viewport};

/// A number rounded to `digits` decimals, as JSON.
fn r(v: f32, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (f64::from(v) * k).round() / k
}

fn v3(v: Vec3, digits: i32) -> Value {
    json!([r(v.x, digits), r(v.y, digits), r(v.z, digits)])
}

/// A step as JSON: 0.1 is 0.1, not 0.100000001.
fn g(v: f32) -> Value {
    menus::fmt_g(v).parse::<f64>().map_or_else(|_| json!(f64::from(v)), |x| json!(x))
}

/// The camera: where it is, which way it looks (degrees: heading, pitch, bank), the view.
fn camera_json(vp: &Viewport) -> Value {
    vp.read(|s| {
        let hpb = s.nav.hpb;
        json!({
            "position": v3(s.nav.position, 3),
            "rotation": [r(hpb.x.to_degrees(), 2), r(hpb.y.to_degrees(), 2), r(hpb.z.to_degrees(), 2)],
            "direction": v3(s.nav.direction(), 3),
            "view": s.nav.current().key(),
            "orthographic": s.nav.ortho,
        })
    })
}

/// What `state` and `ui.list` say of the viewport.
#[must_use]
pub fn view_state(vp: &Viewport) -> Value {
    vp.read(|s| {
        json!({
            "width": s.size[0],
            "height": s.size[1],
            "view": s.nav.current().key(),
            "orthographic": s.nav.ortho,
            "mode": s.prefs.mode.key(),
            "tool": s.gizmo.key(),
            "space": s.prefs.space.key(),
            "gizmo": s.anchor.is_some() && s.gizmo != GizmoType::None,
        })
    })
}

/// Registers the viewport's commands.
pub fn register(reg: &mut Registry) {
    reg.area("viewport", "The level's 3D view: the camera, the view mode, what is shown, snapping, gestures");
    reg.cmd(
        Spec::view(
            "viewport.camera",
            "The viewport's camera: where it is, an axis view, framing objects, camera bookmarks.",
        )
        .example("action=set position=0,20,-30 rotation=0,-30,0"),
        camera,
    );
    reg.cmd(
        Spec::view(
            "viewport.view",
            "How the viewport shows the scene: the view mode, what is shown, the transform tool and its space, snapping, Game Lit's weather, the brush, what Place mode places.",
        )
        .example("mode=game hour=22 show=grid,stats hide=fog tool=move grid_step=0.5"),
        view,
    );
    reg.cmd(
        Spec::view(
            "viewport.play",
            "Plays or stops what moves in the viewport: particle systems, a spawn's startup animation.",
        )
        .example("what=particles action=play objects=campfire_01"),
        play,
    );
    reg.cmd(
        Spec::write(
            "viewport.gesture",
            "A click, a drag, a hover or a drop played in the viewport as the mouse would: what it picked, placed or moved.",
        )
        .example("kind=drop at=400,300 payload=object:props\\box_wood_01"),
        crate::gesture::gesture,
    );
    reg.cmd(
        Spec::read("shot.viewport", "A picture of the viewport's scene as the commands before it left it."),
        shot_viewport,
    );
    reg.cmd(
        Spec::read(
            "perf.frames",
            "The window's frames and the viewport's scene passes since the last ask: rates, times, causes.",
        ),
        perf_frames,
    );
}

// ---- viewport.camera ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CameraAction {
    /// Where the camera is.
    #[default]
    Get,
    /// Moves it: `position`, `rotation`, `view`, `orthographic`.
    Set,
    /// Frames one object (`object`) or the selection.
    Focus,
    /// Frames the scene or the selection (`scope`).
    Frame,
    /// Goes to camera bookmark `slot`, or keeps the camera in it (`store`).
    Bookmark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FrameScope {
    #[default]
    All,
    Selection,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CameraParams {
    /// What to do; left out: get.
    #[serde(default)]
    action: CameraAction,
    /// set: where the camera stands, metres.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// set: which way it looks, degrees: heading, pitch, bank.
    #[serde(default)]
    rotation: Option<[f32; 3]>,
    /// set: an axis view (front, back, left, right, top, bottom) or the free one (perspective).
    #[serde(default)]
    view: Option<String>,
    /// set: an orthographic projection.
    #[serde(default)]
    orthographic: Option<bool>,
    /// focus: the object by name; left out: the selection.
    #[serde(default)]
    object: Option<String>,
    /// focus: select it too.
    #[serde(default)]
    select: bool,
    /// frame: the whole scene (all) or the selection.
    #[serde(default)]
    scope: FrameScope,
    /// bookmark: which, 1 to 9.
    #[serde(default)]
    slot: Option<u8>,
    /// bookmark: keep the camera in the slot instead of going to it.
    #[serde(default)]
    store: bool,
}

async fn camera(ed: Arc<Editor>, p: CameraParams, o: Origin) -> Reply {
    let vp = Viewport::of(&ed);
    match p.action {
        CameraAction::Get => {}
        CameraAction::Set => {
            let target = match p.view.as_deref() {
                Some(v) => Some(Target::parse(v).ok_or_else(|| {
                    CmdError::invalid("view is perspective, front, back, left, right, top or bottom")
                })?),
                None => None,
            };
            if p.position.is_none() && p.rotation.is_none() && target.is_none() && p.orthographic.is_none() {
                return Err(CmdError::invalid("set takes position, rotation, view or orthographic"));
            }
            let finite = |v: [f32; 3], what: &str| {
                if v.iter().all(|x| x.is_finite()) {
                    Ok(Vec3::from_array(v))
                } else {
                    Err(CmdError::invalid(format!("{what} must be three numbers")))
                }
            };
            let position = p.position.map(|v| finite(v, "position")).transpose()?;
            let rotation = p.rotation.map(|v| finite(v, "rotation")).transpose()?;
            vp.with(|s| {
                if let Some(t) = target {
                    s.nav.set_target(t);
                }
                if position.is_some() || rotation.is_some() {
                    let hpb = rotation.map_or(s.nav.hpb, |d| {
                        Vec3::new(d.x.to_radians(), d.y.to_radians(), d.z.to_radians())
                    });
                    let at = position.unwrap_or(s.nav.position);
                    s.nav.set(hpb, at);
                    s.nav.reset_state();
                }
                if let Some(on) = p.orthographic {
                    s.nav.ortho = on;
                }
            });
        }
        CameraAction::Focus => {
            let svc = SceneService::of(&ed)?;
            let snap = svc.store.snapshot();
            let target = match p.object.as_deref() {
                Some(name) => {
                    let obj = snap
                        .find_relaxed(name)
                        .ok_or_else(|| CmdError::not_found(format!("no object '{name}'")))?;
                    if p.select {
                        svc.store
                            .send(SceneCmd::Select { ids: vec![obj.id], mode: SelectMode::Set }, o)
                            .await?;
                    }
                    let (s, lib, obj) = (snap.clone(), svc.library.clone(), obj.clone());
                    Some(ed.exec.cpu(move || crate::draw::object_box(&s, &lib, &obj)).await)
                }
                None => {
                    let (s, lib) = (snap.clone(), svc.library.clone());
                    ed.exec.cpu(move || crate::draw::scene_box(&s, &lib, true)).await
                }
            };
            let b = target.ok_or_else(|| CmdError::invalid("nothing is selected to frame"))?;
            vp.with(|s| {
                let size = s.size;
                s.nav.zoom_extents(&b, size);
            });
        }
        CameraAction::Frame => {
            if !frame(&ed, p.scope == FrameScope::Selection).await {
                return Err(CmdError::invalid(match p.scope {
                    FrameScope::Selection => "nothing is selected to frame",
                    FrameScope::All => "the scene has nothing to frame",
                }));
            }
        }
        CameraAction::Bookmark => {
            let n =
                p.slot.filter(|n| (1..=9).contains(n)).ok_or_else(|| CmdError::invalid("slot is 1 to 9"))?;
            if p.store {
                if !vp.store_bookmark(&ed, n) {
                    return Err(CmdError::invalid("open a scene to keep a camera bookmark of it"));
                }
            } else if !vp.go_to_bookmark(n) {
                return Err(CmdError::not_found(format!("camera bookmark {n} keeps no camera")));
            }
        }
    }
    ed.exec.wake().ping();
    Ok(json!({ "camera": camera_json(&vp), "bookmarks": service::bookmarks(&ed) }))
}

/// Frame All or Frame Selection: the scene's objects (a painting mode's layer), or the selected
/// ones, in view. `false` when there is no box to frame.
pub async fn frame(editor: &Arc<Editor>, selected: bool) -> bool {
    let Ok(svc) = SceneService::of(editor) else { return false };
    let snap = svc.store.snapshot();
    let lib = svc.library.clone();
    let found = editor.exec.cpu(move || crate::draw::scene_box(&snap, &lib, selected)).await;
    let Some(b) = found else { return false };
    Viewport::of(editor).with(|s| {
        let size = s.size;
        s.nav.zoom_extents(&b, size);
    });
    editor.exec.wake().ping();
    true
}

// ---- viewport.view ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Lights {
    /// The lights the last frame of Game Lit drew.
    Drawn,
    /// Every light of the level as Game Lit sees it.
    All,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// The view mode: studio, unlit, game (Game Lit) or wireframe.
    #[serde(default)]
    mode: Option<String>,
    /// The transform tool: select, move, rotate or scale.
    #[serde(default)]
    tool: Option<String>,
    /// The gizmo's axes: world or local.
    #[serde(default)]
    space: Option<String>,
    /// Show pill switches to turn on: grid, safe_rect, stats, spawn_helpers, spawn_names,
    /// light_names, glow_crosses, ai_map, fog.
    #[serde(default)]
    show: Option<Vec<String>>,
    /// Show pill switches to turn off.
    #[serde(default)]
    hide: Option<Vec<String>>,
    /// Moves go in steps of the grid.
    #[serde(default)]
    grid_snap: Option<bool>,
    /// Turns go in steps of the angle.
    #[serde(default)]
    angle_snap: Option<bool>,
    /// Scaling goes in steps.
    #[serde(default)]
    scale_snap: Option<bool>,
    /// A placed or dropped thing lands on the surface under the pointer.
    #[serde(default)]
    surface_snap: Option<bool>,
    /// ... and stands on its slope.
    #[serde(default)]
    surface_align: Option<bool>,
    /// ... on the nearest vertex of what it lands on.
    #[serde(default)]
    vertex_snap: Option<bool>,
    /// The grid step, metres (0.01 0.05 0.1 0.25 0.5 1 2 5 10, or any above 0).
    #[serde(default)]
    grid_step: Option<f32>,
    /// The angle step, degrees (5 10 15 22.5 30 45 90, or any above 0).
    #[serde(default)]
    angle_step: Option<f32>,
    /// The scale step, a factor (0.05 0.1 0.25 0.5 1 2, or any above 0).
    #[serde(default)]
    scale_step: Option<f32>,
    /// Game Lit: the weather by its file's name; empty is the default for the hour.
    #[serde(default)]
    weather: Option<String>,
    /// Game Lit: the hour, 0 to 24.
    #[serde(default)]
    hour: Option<f32>,
    /// Game Lit: 0 is the game's tonemap, else a factor.
    #[serde(default)]
    exposure: Option<f32>,
    /// Game Lit: how much of the sky's light reaches under a roof, 0 to 1.
    #[serde(default)]
    indoor_hemi: Option<f32>,
    /// Game Lit: bake the map of what is under a roof again.
    #[serde(default)]
    rebake: bool,
    /// Game Lit: read the weather and the game's render settings again.
    #[serde(default)]
    reload: bool,
    /// Answer the linked game's weathers.
    #[serde(default)]
    weathers: bool,
    /// Answer Game Lit's lights: the ones drawn, or every light of the level.
    #[serde(default)]
    lights: Option<Lights>,
    /// The Paint mode's brush: the detail mesh it paints.
    #[serde(default)]
    brush_mesh: Option<String>,
    /// The brush's radius, metres.
    #[serde(default)]
    brush_radius: Option<f32>,
    /// The brush's density, 0 to 1.
    #[serde(default)]
    brush_density: Option<f32>,
    /// What Place mode places: `<kind>` or `<kind>:<reference>` (`object:props\box_01`,
    /// `spawn:stalker`, `light`); arming puts Place mode up. Empty: nothing.
    #[serde(default)]
    arm: Option<String>,
    /// The viewport draws every frame.
    #[serde(default)]
    real_time: Option<bool>,
    /// The scene's sounds stay silent.
    #[serde(default)]
    mute_sounds: Option<bool>,
    /// The graphics adapter by its name for the next start; empty or `default`: the system's.
    #[serde(default)]
    gpu: Option<String>,
}

fn step(value: Option<f32>, what: &str) -> Result<Option<f32>, CmdError> {
    match value {
        Some(v) if !(v.is_finite() && v > 0.0) => {
            Err(CmdError::invalid(format!("{what} must be a step above 0")))
        }
        other => Ok(other),
    }
}

fn show_keys(list: Option<&[String]>) -> Result<Vec<&str>, CmdError> {
    let mut out = Vec::new();
    for k in list.unwrap_or_default() {
        let k = k.trim();
        if !SHOW_FLAGS.iter().any(|f| f.key == k) {
            let all: Vec<&str> = SHOW_FLAGS.iter().map(|f| f.key).collect();
            return Err(CmdError::invalid(format!(
                "'{k}' is not shown or hidden; there are {}",
                all.join(", ")
            )));
        }
        out.push(k);
    }
    Ok(out)
}

async fn view(ed: Arc<Editor>, p: ViewParams, _o: Origin) -> Reply {
    let vp = Viewport::of(&ed);
    vp.sync(&ed);
    // everything is read first: a request with a bad value changes nothing
    let mode = p
        .mode
        .as_deref()
        .map(|m| {
            ViewMode::parse(m).ok_or_else(|| CmdError::invalid("mode is studio, unlit, game or wireframe"))
        })
        .transpose()?;
    let tool = p
        .tool
        .as_deref()
        .map(|t| {
            GizmoType::parse(t).ok_or_else(|| CmdError::invalid("tool is select, move, rotate or scale"))
        })
        .transpose()?;
    let space = match p.space.as_deref().map(str::trim) {
        Some("world") => Some(Space::World),
        Some("local") => Some(Space::Local),
        Some(_) => return Err(CmdError::invalid("space is world or local")),
        None => None,
    };
    let (show, hide) = (show_keys(p.show.as_deref())?, show_keys(p.hide.as_deref())?);
    let steps = [
        (GizmoType::Move, step(p.grid_step, "grid_step")?),
        (GizmoType::Rotate, step(p.angle_step, "angle_step")?),
        (GizmoType::Scale, step(p.scale_step, "scale_step")?),
    ];
    let armed = match p.arm.as_deref().map(str::trim) {
        Some("" | "none") => Some(None),
        Some(key) => Some(Some(Placeable::parse(key).ok_or_else(|| {
            CmdError::invalid("arm is <kind> or <kind>:<reference>, the kind one scene.place takes")
        })?)),
        None => None,
    };
    let sup = ed
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map_or([true; 3], |svc| gizmo::supports_of(&svc.store.snapshot()));
    if let Some(t) = tool {
        let i = match t {
            GizmoType::Move => Some(0),
            GizmoType::Rotate => Some(1),
            GizmoType::Scale => Some(2),
            GizmoType::None => None,
        };
        if i.is_some_and(|i| !sup[i]) {
            return Err(CmdError::invalid(format!(
                "the selection cannot {}: not every selected object takes it",
                t.key()
            )));
        }
    }
    let gpu = match p.gpu.as_deref() {
        Some(n) if n.is_empty() || n.eq_ignore_ascii_case("default") => Some(String::new()),
        Some(n) => {
            let known = gz_app::gpu::Gpu::of(&ed).adapters().is_none_or(|a| a.names.iter().any(|x| x == n));
            if !known {
                return Err(CmdError::invalid(format!(
                    "no graphics adapter '{n}' (render.gpu.adapters lists them)"
                )));
            }
            Some(n.to_owned())
        }
        None => None,
    };

    if let Some(m) = mode {
        service::set_mode(&ed, m);
    }
    if let Some(t) = tool {
        vp.with(|s| s.gizmo = t);
    }
    if let Some(s) = space {
        service::set_space(&ed, s);
    }
    for k in show {
        service::set_show(&ed, k, true);
    }
    for k in hide {
        service::set_show(&ed, k, false);
    }
    for (key, v) in [
        ("grid", p.grid_snap),
        ("angle", p.angle_snap),
        ("scale", p.scale_snap),
        ("surface", p.surface_snap),
        ("align", p.surface_align),
        ("vertex", p.vertex_snap),
    ] {
        if let Some(on) = v {
            service::set_snap(&ed, key, on);
        }
    }
    for (kind, v) in steps {
        if let Some(v) = v {
            service::set_step(&ed, kind, v);
        }
    }
    if p.brush_mesh.is_some() || p.brush_radius.is_some() || p.brush_density.is_some() {
        let mut b = service::brush(&ed);
        if let Some(m) = &p.brush_mesh {
            b.mesh = Some(m.trim().replace('/', "\\").into()).filter(|m: &gz_editor::SmolStr| !m.is_empty());
        }
        if let Some(r) = p.brush_radius {
            b.radius = r.clamp(0.5, 200.0);
        }
        if let Some(d) = p.brush_density {
            b.density = d.clamp(0.0, 1.0);
        }
        service::set_brush(&ed, b);
    }
    if let Some(a) = armed {
        crate::place::arm(&ed, a);
    }
    if let Some(on) = p.real_time {
        service::set_real_time(&ed, on);
    }
    if let Some(on) = p.mute_sounds {
        service::set_mute_sounds(&ed, on);
    }
    if let Some(name) = gpu {
        gz_app::EditorSettings::of(&ed).set_gpu_adapter(&name);
    }
    service::save(&ed);
    // Game Lit
    let mut weather_changed = false;
    if let Some(w) = &p.weather {
        let w = w.trim().to_owned();
        vp.with(|s| s.game.weather = w);
        weather_changed = true;
    }
    if let Some(h) = p.hour {
        vp.with(|s| s.game.hour = h.rem_euclid(24.0));
        weather_changed = true;
    }
    if let Some(e) = p.exposure {
        vp.with(|s| s.game.exposure = e.max(0.0));
    }
    if let Some(i) = p.indoor_hemi {
        vp.with(|s| s.game.indoor_hemi = i.clamp(0.0, 1.0));
    }
    if p.reload || weather_changed || (p.weathers && vp.read(|s| s.game.weathers.is_empty())) {
        vp.load_weather(&ed, p.reload).await;
    }
    if p.rebake {
        vp.with(|s| {
            s.game.rebake = true;
            s.game.roof = None;
        });
    }
    if mode == Some(ViewMode::Game) && vp.read(|s| !s.game.weather_ok) {
        vp.load_weather(&ed, true).await;
    }
    // the lights the answer names are the next frame's
    if vp.read(|s| s.game.active())
        && (p.lights.is_some() || weather_changed)
        && let Some(rend) = vp.renderer_any(&ed)
    {
        vp.settle().await;
        if let Some(packet) = vp.make_packet(&ed, &rend, false, service::Reason::Picture).await {
            rend.submit(packet);
        }
    }
    ed.exec.wake().ping();

    let mut out = view_answer(&ed, &vp, sup);
    if let Value::Object(m) = &mut out {
        if p.weathers {
            m.insert("weathers".into(), json!(vp.read(|s| s.game.weathers.clone())));
        }
        if let Some(which) = p.lights {
            let list: Vec<Value> = match which {
                Lights::Drawn => vp.read(|s| s.game.drawn.iter().map(light_json).collect()),
                Lights::All => match SceneService::of(&ed) {
                    Ok(svc) => {
                        let snap = svc.store.snapshot();
                        let data = vp.game_data_for(&ed).await;
                        let lights = ed
                            .exec
                            .cpu(move || lighting::collect(&snap, data.0.as_deref(), data.1.as_deref()))
                            .await;
                        lights.iter().map(light_json).collect()
                    }
                    Err(_) => Vec::new(),
                },
            };
            m.insert("lights".into(), Value::Array(list));
        }
    }
    let failed =
        vp.read(|s| s.game.enabled && !s.game.loading && !s.game.weather_ok && !s.game.error.is_empty());
    if failed && (mode == Some(ViewMode::Game) || weather_changed) {
        let text = vp.read(|s| s.game.error.clone());
        return Err(
            CmdError::new("failed", format!("Game Lit could not read the weather: {text}")).with_data(out)
        );
    }
    Ok(out)
}

/// `viewport.view`'s answer: everything it sets, as it is now.
fn view_answer(ed: &Arc<Editor>, vp: &Viewport, sup: [bool; 3]) -> Value {
    let settings = gz_app::EditorSettings::of(ed);
    let gpu = gz_app::gpu::Gpu::of(ed);
    let adapters = gpu.adapters().unwrap_or_default();
    let render = json!({
        "scale": vp.read(|s| (s.prefs.render_scale * 100.0).round() as i64),
        "real_time": service::real_time(ed),
        "mute_sounds": settings.render_flag("mute_sounds"),
        "gpu": { "active": adapters.active, "adapter": settings.gpu_adapter(), "adapters": adapters.names },
    });
    let armed = crate::place::armed(ed).map(|a| a.key());
    vp.read(|s| {
        let show: serde_json::Map<String, Value> =
            SHOW_FLAGS.iter().map(|f| (f.key.to_owned(), json!((f.get)(&s.prefs.show)))).collect();
        let snap = |k: GizmoType| json!({ "on": s.snap_on(k), "step": g(s.step(k)) });
        let gl = &s.game;
        let Brush { mesh, radius, density } = &s.brush;
        json!({
            "mode": s.prefs.mode.key(),
            "tool": s.gizmo.key(),
            "space": s.prefs.space.key(),
            "show": show,
            "grid": snap(GizmoType::Move),
            "angle": snap(GizmoType::Rotate),
            "scale": snap(GizmoType::Scale),
            "steps": { "grid": MOVE_STEPS, "angle": ANGLE_STEPS, "scale": SCALE_STEPS },
            "surface_snap": s.prefs.snaps.surface,
            "surface_align": s.prefs.snaps.align,
            "vertex_snap": s.prefs.snaps.vertex,
            "supports": { "move": sup[0], "rotate": sup[1], "scale": sup[2] },
            "armed": armed,
            "brush": { "mesh": mesh, "radius": r(*radius, 2), "density": r(*density, 2) },
            "paint_layer": s.paint_layer.word(),
            "game": {
                "weather": gl.weather,
                "hour": r(gl.hour, 2),
                "clock": hour_text(gl.hour),
                "weather_ok": gl.weather_ok,
                "loading": gl.loading,
                "error": gl.error,
                "exposure": r(gl.exposure_now(), 3),
                "exposure_setting": r(gl.exposure, 3),
                "indoor_hemi": r(gl.indoor_hemi, 3),
                "sun_dir": v3(gl.at.sun_dir, 4),
                "fog_color": v3(gl.at.fog_color, 4),
                "lights_drawn": gl.drawn.len(),
                "lights_in_view": gl.in_view,
                "roof": gl.roof.as_ref().map(|m| json!({ "cells": m.cells, "origin": [r(m.origin[0], 1), r(m.origin[1], 1)], "bake_ms": gl.roof_bake_ms })),
            },
            "render": render,
        })
    })
}

fn light_json(l: &lighting::GameLight) -> Value {
    json!({ "kind": l.kind, "owner": l.owner, "position": v3(l.position, 2), "range": r(l.range, 2), "color": v3(l.color, 3), "spot_cos": r(l.spot_cos, 3) })
}

// ---- viewport.play ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PlayWhat {
    /// Particle systems.
    Particles,
    /// A spawn's startup animation (its model's motion).
    Animation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PlayAction {
    /// What plays.
    #[default]
    State,
    Play,
    Stop,
    /// An animation: pauses it, or goes on with a paused one.
    Pause,
    /// An animation: its model at its first frame.
    First,
    /// An animation: its model at its last frame.
    Last,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlayParams {
    /// What plays.
    what: PlayWhat,
    /// What to do; left out: say what plays.
    #[serde(default)]
    action: PlayAction,
    /// The objects by name: particle systems (left out: every one), or the one spawn of an
    /// animation (left out: the selected one).
    #[serde(default)]
    objects: Option<Vec<String>>,
}

async fn play(ed: Arc<Editor>, p: PlayParams, _o: Origin) -> Reply {
    let names = p.objects.unwrap_or_default();
    match p.what {
        PlayWhat::Particles => {
            let on = match p.action {
                PlayAction::Play => Some(true),
                PlayAction::Stop => Some(false),
                PlayAction::State => None,
                _ => return Err(CmdError::invalid("particles play, stop or say their state")),
            };
            crate::draw_particles::play_command(&ed, &names, on).await
        }
        PlayWhat::Animation => {
            let object = match names.as_slice() {
                [one] => one.clone(),
                [] => {
                    let svc = SceneService::of(&ed)?;
                    let snap = svc.store.snapshot();
                    let sel = snap.selected_in_order();
                    let [id] = sel.as_slice() else {
                        return Err(CmdError::invalid("name the spawn, or select one"));
                    };
                    snap.get(*id).map(|o| o.name.to_string()).unwrap_or_default()
                }
                _ => return Err(CmdError::invalid("an animation plays on one spawn at a time")),
            };
            let action = match p.action {
                PlayAction::State => "state",
                PlayAction::Play => "play",
                PlayAction::Stop => "stop",
                PlayAction::Pause => "pause",
                PlayAction::First => "first",
                PlayAction::Last => "last",
            };
            crate::spawn_anim::animation_command(&ed, &object, action).await
        }
    }
}

// ---- shot.viewport, perf.frames -----------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoParams {}

async fn shot_viewport(ed: Arc<Editor>, _p: NoParams, _o: Origin) -> Reply {
    let vp = Viewport::of(&ed);
    let (image, pending) = vp.picture(&ed).await?;
    let (w, h) = (image.width, image.height);
    let png = ed.exec.cpu(move || gz_app::encode_png(image.width, image.height, &image.rgba)).await?;
    let encoded = ed.exec.cpu(move || base64::engine::general_purpose::STANDARD.encode(png)).await;
    let scale = vp.read(|s| r(s.ppp, 3));
    // what was still loading when the wait ran out: never a silent half picture
    let mut out = json!({ "png_base64": encoded, "width": w, "height": h, "scale": scale });
    if !pending.is_empty()
        && let Value::Object(m) = &mut out
    {
        m.insert("pending".into(), json!(pending));
    }
    Ok(out)
}

async fn perf_frames(ed: Arc<Editor>, _p: NoParams, _o: Origin) -> Reply {
    let settings = gz_app::EditorSettings::of(&ed);
    let stamp = ed.frames.current();
    let vp = Viewport::of(&ed);
    let p = vp.pacing(stamp.frame, ed.frames.causes());
    let since = p.since;
    let times: Vec<_> = since
        .map(|f| ed.frames.times_after(f).into_iter().filter(|t| t.frame <= stamp.frame).collect())
        .unwrap_or_default();
    let times = frame_times(&times);
    let by_reason: serde_json::Map<String, Value> =
        p.by_reason.iter().map(|(r, n)| (r.key().to_owned(), json!(n))).collect();
    let last_pass = p
        .last_pass
        .map(|(r, at)| json!({ "reason": r.key(), "ago_ms": (at.elapsed().as_secs_f64() * 1000.0).round() }));
    Ok(json!({
        "active_idle_fps": settings.pref("active_idle_fps").map_or(120, |v| v as u64),
        "measured_fps": (p.fps * 1000.0).round() / 1000.0,
        "measured_frame_ms": (p.frame_ms * 1000.0).round() / 1000.0,
        "window": ed.frames.has_window(),
        // a particle system in view plays, or Real Time is on: the viewport draws every frame
        "realtime_render": vp.animating() || service::real_time(&ed),
        "frames": stamp.frame,
        // the frames presented since the last ask: their own work apart from the wait for the
        // presentation - a frame that waited for vsync is not a slow frame
        "sampled_frames": times.count,
        "frame_work_ms": { "mean": times.work_mean, "p95": times.work_p95, "max": times.work_max },
        "frame_wait_ms": { "mean": times.wait_mean, "max": times.wait_max },
        "slow_frames": times.slow,
        "window_seconds": (p.seconds * 1000.0).round() / 1000.0,
        "ui_frames": since.map_or(0, |f| stamp.frame.saturating_sub(f)),
        "ui_frame_causes": p.causes.iter().take(8).map(|(k, n)| json!([k.as_str(), n])).collect::<Vec<_>>(),
        "scene_passes": p.passes,
        "scene_pass_reasons": by_reason,
        "last_scene_pass": last_pass,
    }))
}

/// What `perf.frames` says of the frames since its last ask.
#[derive(Default)]
struct FrameTimes {
    count: usize,
    work_mean: f64,
    work_p95: f64,
    work_max: f64,
    wait_mean: f64,
    wait_max: f64,
    /// Frames whose own work took more than a 60 Hz frame.
    slow: usize,
}

fn frame_times(t: &[gz_editor::FrameTime]) -> FrameTimes {
    if t.is_empty() {
        return FrameTimes::default();
    }
    let ms = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    let mut work: Vec<f32> = t.iter().map(|x| x.work_ms).collect();
    work.sort_by(f32::total_cmp);
    let n = t.len();
    let mean = |f: &dyn Fn(&gz_editor::FrameTime) -> f32| ms(t.iter().map(f).sum::<f32>() / n as f32);
    FrameTimes {
        count: n,
        work_mean: mean(&|x| x.work_ms),
        work_p95: ms(work.get((n * 95 / 100).min(n - 1)).copied().unwrap_or(0.0)),
        work_max: ms(work.last().copied().unwrap_or(0.0)),
        wait_mean: mean(&|x| x.wait_ms),
        wait_max: ms(t.iter().map(|x| x.wait_ms).fold(0.0, f32::max)),
        slow: t.iter().filter(|x| x.work_ms > 1000.0 / 60.0).count(),
    }
}
