//! The viewport's commands over MCP; the wire is `LevelMcpScene.cpp` (`camera_get`,
//! `camera_set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`),
//! `GameLightingScene.cpp` (`game_lighting`), `LevelMcp.cpp` (`frame_pacing`) and
//! `GrizzlyMCP.cpp` (`screenshot_viewport`).

use std::sync::Arc;

use base64::Engine;
use glam::Vec3;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, Registry, Reply, Value, json};
use gz_scene::{SceneCmd, SceneService};
use gz_scene::apply::SelectMode;

use crate::camera::Target;
use crate::gizmo::{self, GizmoType};
use crate::lighting::{self, hour_text};
use crate::menus;
use crate::pane::tool_title;
use crate::service::{Action, Fill, Viewport};

const AREA: &str = "gz-ui-viewport";

/// `%.Nf` as a JSON number.
fn r(v: f32, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (f64::from(v) * k).round() / k
}

fn v3(v: Vec3, digits: i32) -> Value {
    json!([r(v.x, digits), r(v.y, digits), r(v.z, digits)])
}

/// `%g` of a float as a JSON number: 0.1 is 0.1, not 0.100000001.
fn g(v: f32) -> Value {
    menus::fmt_g(v).parse::<f64>().map_or_else(|_| json!(f64::from(v)), |x| json!(x))
}

/// `AppendCameraJson`.
fn camera_json(vp: &Viewport) -> Value {
    vp.read(|s| json!({ "position": v3(s.nav.position, 3), "hpb": v3(s.nav.hpb, 4), "direction": v3(s.nav.direction(), 3) }))
}

/// What `state` and `ui_list` say of the viewport.
#[must_use]
pub fn view_state(vp: &Viewport) -> Value {
    vp.read(|s| {
        json!({
            "width": s.size[0],
            "height": s.size[1],
            "view": s.nav.current().key(),
            "orthographic": s.nav.ortho,
            "camera": { "position": v3(s.nav.position, 3), "hpb": v3(s.nav.hpb, 4), "direction": v3(s.nav.direction(), 3) },
            "tool": tool_name(s.tool.action, s.tool.gizmo),
            "gizmo": s.anchor.is_some() && s.tool.action == Action::Select && s.tool.gizmo != GizmoType::None,
            "fill": s.view.fill.key(),
            "lighting": s.view.lighting,
            "grid": s.view.grid,
            "safe_rect": s.view.safe_rect,
            "stats": s.view.stats,
            "helpers": s.view.helpers,
            "game_lighting": s.game.enabled,
        })
    })
}

fn tool_name(action: Action, gizmo: GizmoType) -> &'static str {
    match (action, gizmo) {
        (Action::Place, _) => "place",
        (_, GizmoType::Move) => "move",
        (_, GizmoType::Rotate) => "rotate",
        (_, GizmoType::Scale) => "scale",
        _ => "select",
    }
}

/// Registers `camera_get`, `camera_set`, `focus_object`, `viewport_navigation`,
/// `viewport_controls`, `view_mode`, `game_lighting`, `frame_pacing`, `screenshot_viewport`.
pub fn register(reg: &mut Registry) {
    reg.add_sync(AREA, "camera_get", |ed, _a, _o| Ok(json!({ "camera": camera_json(&Viewport::of(ed)) })));
    reg.add(AREA, "camera_set", |ed, a, _o| async move { camera_set(&ed, &a) });
    reg.add(AREA, "focus_object", focus_object);
    reg.add(AREA, "viewport_navigation", viewport_navigation);
    reg.add(AREA, "viewport_controls", |ed, a, _o| async move { viewport_controls(&ed, &a) });
    reg.add(AREA, "view_mode", |ed, a, _o| async move { view_mode(&ed, &a).await });
    reg.add(AREA, "game_lighting", |ed, a, _o| async move { game_lighting(&ed, &a).await });
    reg.add(AREA, "frame_pacing", |ed, a, _o| async move { frame_pacing(&ed, &a) });
    reg.add(AREA, "screenshot_viewport", |ed, _a, _o| async move { screenshot_viewport(&ed).await });
}

/// A change made over MCP is drawn: the pane's next frame builds the pass.
fn redraw(ed: &Editor) {
    ed.exec.wake().ping();
}

fn camera_set(ed: &Arc<Editor>, a: &Args) -> Reply {
    let vp = Viewport::of(ed);
    let (mut pos, mut hpb) = vp.read(|s| (s.nav.position, s.nav.hpb));
    let mut any = false;
    for (i, k) in ["x", "y", "z"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            pos[i] = v;
            any = true;
        }
    }
    for (i, k) in ["h", "p", "b"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            hpb[i] = v;
            any = true;
        }
    }
    if !any {
        return Err(CmdError::invalid("pass at least one of x,y,z,h,p,b"));
    }
    vp.with(|s| {
        s.nav.set(hpb, pos);
        s.nav.reset_state();
    });
    redraw(ed);
    Ok(json!({ "camera": camera_json(&vp) }))
}

async fn focus_object(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let name = a.str("name").ok_or_else(|| CmdError::invalid("missing 'name' argument"))?;
    let svc = SceneService::of(&ed)?;
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).ok_or_else(|| CmdError::not_found("object not found"))?.clone();
    if a.bool("select").unwrap_or(false) {
        // exactly like a manual pick: clear, select, switch the target
        svc.store.send(SceneCmd::SelectClass { class: None, select: false }, o).await?;
        svc.store.send(SceneCmd::Select { ids: vec![obj.id], mode: SelectMode::Add }, o).await?;
        if snap.tools.target != obj.class {
            svc.store.send(SceneCmd::SetTarget(obj.class), o).await?;
        }
    }
    let lib = svc.library.clone();
    let s = snap.clone();
    let b = ed.exec.cpu(move || crate::draw::object_box(&s, &lib, &obj)).await;
    let vp = Viewport::of(&ed);
    vp.with(|s| {
        let size = s.size;
        s.nav.zoom_extents(&b, size);
    });
    redraw(&ed);
    let p = vp.read(|s| s.nav.position);
    Ok(json!({ "camera": v3(p, 2) }))
}

async fn viewport_navigation(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let vp = Viewport::of(&ed);
    let view = a.str("view").unwrap_or("");
    let scope = a.str("scope").unwrap_or("");
    let action = a.str("action").filter(|s| !s.is_empty()).unwrap_or(if view.is_empty() { "get" } else { "set" });
    if action.eq_ignore_ascii_case("set") {
        let Some(t) = Target::parse(view) else {
            return Err(CmdError::invalid("view must be perspective, front, back, left, right, top, or bottom"));
        };
        vp.with(|s| s.nav.set_target(t));
        redraw(&ed);
    } else if action.eq_ignore_ascii_case("frame") {
        let selection = scope.eq_ignore_ascii_case("selection");
        let bad = !selection && !scope.is_empty() && !scope.eq_ignore_ascii_case("all");
        if bad || !crate::keys::frame(&ed, selection).await {
            return Err(CmdError::invalid("scope must be all or selection, and a scene must be ready"));
        }
    } else if !action.eq_ignore_ascii_case("get") {
        return Err(CmdError::invalid("action must be get, set, or frame"));
    }
    let view = vp.read(|s| s.nav.current().key());
    Ok(json!({ "view": view, "camera": camera_json(&vp) }))
}

/// The viewport's right pills. Arguments that are left out keep their value; a request with a
/// bad one changes nothing.
fn viewport_controls(ed: &Arc<Editor>, a: &Args) -> Reply {
    let vp = Viewport::of(ed);
    let class = ed.project_opt().and_then(|s| s.services.get::<SceneService>()).map_or(gz_scene::ObjClass::SceneObject, |svc| svc.store.snapshot().tools.target);
    let [sup_move, sup_rotate, sup_scale] = gizmo::supports(class);
    let tool_arg = a.str("tool").filter(|t| !t.is_empty());
    let (mut action, mut kind) = vp.read(|s| (s.tool.action, s.tool.gizmo));
    if let Some(t) = tool_arg {
        action = Action::Select;
        kind = GizmoType::None;
        let supported = if t.eq_ignore_ascii_case("place") {
            action = Action::Place;
            true
        } else if t.eq_ignore_ascii_case("move") {
            kind = GizmoType::Move;
            sup_move
        } else if t.eq_ignore_ascii_case("rotate") {
            kind = GizmoType::Rotate;
            sup_rotate
        } else if t.eq_ignore_ascii_case("scale") {
            kind = GizmoType::Scale;
            sup_scale
        } else if t.eq_ignore_ascii_case("select") {
            true
        } else {
            return Err(CmdError::invalid("tool must be select, place, move, rotate or scale"));
        };
        if !supported {
            return Err(CmdError::invalid(format!("the {} tool cannot {t} objects", tool_title(class))));
        }
    }
    let snaps = [("grid", "grid_snap", GizmoType::Move), ("scale", "scale_snap", GizmoType::Scale), ("angle", "angle_snap", GizmoType::Rotate)];
    let mut wanted = Vec::with_capacity(3);
    for (name, toggle, k) in snaps {
        let (step, on) = vp.read(|s| (s.step(k), s.snap_on(k)));
        let given = a.f32(name);
        if let Some(v) = given
            && !(v > 0.0)
        {
            return Err(CmdError::invalid(format!("{name} must be a step above 0")));
        }
        wanted.push((k, given.unwrap_or(step), given.is_some(), a.bool(toggle).unwrap_or(on)));
    }
    if tool_arg.is_some() {
        vp.with(|s| {
            s.tool.action = action;
            s.tool.gizmo = kind;
        });
    }
    for (k, step, has_step, on) in &wanted {
        if *has_step {
            menus::set_step(ed, *k, *step);
        }
        vp.with(|s| match k {
            GizmoType::Move => s.tool.grid_snap = *on,
            GizmoType::Scale => s.tool.scale_snap = *on,
            _ => s.tool.angle_snap = *on,
        });
    }
    vp.with(|s| {
        s.settings.object_snap = a.bool("object_snap").unwrap_or(s.settings.object_snap);
        s.settings.vertex_snap = a.bool("vertex_snap").unwrap_or(s.settings.vertex_snap);
        s.settings.normal_align = a.bool("normal_align").unwrap_or(s.settings.normal_align);
    });
    redraw(ed);
    vp.read(|s| {
        let snap = |k: GizmoType| json!({ "on": s.snap_on(k), "step": g(s.step(k)) });
        Ok(json!({
            "tool": tool_name(s.tool.action, s.tool.gizmo),
            "grid": snap(GizmoType::Move),
            "scale": snap(GizmoType::Scale),
            "angle": snap(GizmoType::Rotate),
            "object_snap": s.settings.object_snap,
            "vertex_snap": s.settings.vertex_snap,
            "normal_align": s.settings.normal_align,
            "supports": { "move": sup_move, "rotate": sup_rotate, "scale": sup_scale },
        }))
    })
}

const PRESETS: [&str; 5] = ["unlit", "lit", "game", "wireframe", "point"];

async fn view_mode(ed: &Arc<Editor>, a: &Args) -> Reply {
    let vp = Viewport::of(ed);
    let mut game_on: Option<bool> = None;
    // presets first, so explicit fields can still override them
    if let Some(preset) = a.str("preset").filter(|p| !p.is_empty()) {
        match preset.to_ascii_lowercase().as_str() {
            "lit" => vp.with(|s| {
                s.view.fill = Fill::Solid;
                s.view.lighting = true;
                s.view.textures = true;
            }),
            "unlit" => vp.with(|s| {
                s.view.fill = Fill::Solid;
                s.view.lighting = false;
                s.view.textures = true;
            }),
            "game" => {
                vp.with(|s| {
                    s.view.fill = Fill::Solid;
                    s.view.textures = true;
                });
                game_on = Some(true);
            }
            "wireframe" => vp.with(|s| s.view.fill = Fill::Wireframe),
            "point" => vp.with(|s| s.view.fill = Fill::Point),
            _ => return Err(CmdError::invalid("unknown preset").with_data(json!({ "presets": PRESETS }))),
        }
    }
    if let Some(fill) = a.str("fill").filter(|f| !f.is_empty()) {
        let f = match fill.to_ascii_lowercase().as_str() {
            "solid" => Fill::Solid,
            "wireframe" => Fill::Wireframe,
            "point" => Fill::Point,
            _ => return Err(CmdError::invalid("fill must be solid, wireframe or point")),
        };
        vp.with(|s| s.view.fill = f);
    }
    if let Some(shade) = a.str("shade").filter(|f| !f.is_empty()) {
        let flat = match shade.to_ascii_lowercase().as_str() {
            "flat" => true,
            "gouraud" => false,
            _ => return Err(CmdError::invalid("shade must be flat or gouraud")),
        };
        vp.with(|s| s.view.flat = flat);
    }
    vp.with(|s| {
        let v = &mut s.view;
        for (name, slot) in [
            ("lighting", &mut v.lighting),
            ("textures", &mut v.textures),
            ("edged_faces", &mut v.edged_faces),
            ("filter_linear", &mut v.filter_linear),
            ("fog", &mut v.fog),
            ("environment", &mut v.environment),
            ("grid", &mut v.grid),
            ("safe_rect", &mut v.safe_rect),
            ("stats", &mut v.stats),
            ("helpers", &mut v.helpers),
        ] {
            if let Some(b) = a.bool(name) {
                *slot = b;
            }
        }
    });
    if let Some(b) = a.bool("game_lighting") {
        game_on = Some(b);
    }
    if let Some(on) = game_on {
        let need = vp.with(|s| {
            let need = on && !s.game.enabled && !s.game.weather_ok;
            s.game.enabled = on;
            need
        });
        if need {
            vp.load_weather(ed, true).await;
        }
    }
    redraw(ed);
    vp.read(|s| {
        let v = &s.view;
        Ok(json!({
            "fill": v.fill.key(),
            "shade": if v.flat { "flat" } else { "gouraud" },
            "lighting": v.lighting,
            "textures": v.textures,
            "edged_faces": v.edged_faces,
            "filter_linear": v.filter_linear,
            "fog": v.fog,
            "environment": v.environment,
            "grid": v.grid,
            "safe_rect": v.safe_rect,
            "stats": v.stats,
            "helpers": v.helpers,
            "game_lighting": s.game.enabled,
            "presets": PRESETS,
        }))
    })
}

/// `GameLighting_JsonState`.
fn game_state(vp: &Viewport) -> Value {
    vp.read(|s| {
        let gl = &s.game;
        let mut counts = [0u32; 4];
        for l in &gl.drawn {
            let i = match l.kind {
                "level" => 0,
                "lamp" => 1,
                "lamp_ambient" => 2,
                _ => 3,
            };
            if let Some(c) = counts.get_mut(i) {
                *c += 1;
            }
        }
        let st = &gl.settings;
        let (cells, origin) = gl.roof.as_ref().map_or((0, [0.0, 0.0]), |m| (m.cells, m.origin));
        json!({
            "enabled": gl.enabled,
            "weather": gl.weather,
            "hour": r(gl.hour, 2),
            "clock": hour_text(gl.hour),
            "weather_ok": gl.weather_ok,
            "error": gl.error,
            "ambient": v3(gl.at.ambient, 4),
            "hemi": v3(gl.at.hemi, 4),
            "sun_color": v3(gl.at.sun_color, 4),
            "sun_dir": v3(gl.at.sun_dir, 4),
            "fog_color": v3(gl.at.fog_color, 4),
            "lumscale": { "sun": r(st.lumscale_sun, 3), "hemi": r(st.lumscale_hemi, 3), "amb": r(st.lumscale_amb, 3) },
            "tonemap": { "on": st.tonemap, "middlegray": r(st.tonemap_middlegray, 3), "lowlum": r(st.tonemap_lowlum, 4), "amount": r(st.tonemap_amount, 3) },
            "exposure": r(gl.exposure_now(), 3),
            "exposure_setting": r(gl.exposure, 3),
            "auto_exposure": r(gl.auto_exposure, 3),
            "scene_lum": r(gl.scene_lum, 4),
            "indoor_hemi": r(gl.indoor_hemi, 3),
            "lights": { "drawn": gl.drawn.len(), "in_view": gl.in_view, "level": counts[0], "lamp": counts[1], "lamp_ambient": counts[2], "campfire": counts[3] },
            "roof": { "cells": cells, "cell": r(lighting::ROOF_CELL, 2), "origin": [r(origin[0], 1), r(origin[1], 1)], "bake_ms": gl.roof_bake_ms },
        })
    })
}

fn light_json(l: &lighting::GameLight) -> Value {
    json!({ "kind": l.kind, "owner": l.owner, "pos": v3(l.position, 2), "range": r(l.range, 2), "color": v3(l.color, 3), "spot_cos": r(l.spot_cos, 3) })
}

async fn game_lighting(ed: &Arc<Editor>, a: &Args) -> Reply {
    let vp = Viewport::of(ed);
    let mut weather_changed = false;
    if let Some(w) = a.get("weather").and_then(Value::as_str) {
        // empty = the default for the hour (the first night of a new game)
        let w = w.to_owned();
        vp.with(|s| s.game.weather = w);
        weather_changed = true;
    }
    if let Some(h) = a.f32("hour") {
        vp.with(|s| s.game.hour = h);
        weather_changed = true;
    }
    if let Some(e) = a.f32("exposure") {
        vp.with(|s| s.game.exposure = e.max(0.0));
    }
    if let Some(i) = a.f32("indoor_hemi") {
        vp.with(|s| s.game.indoor_hemi = i.clamp(0.0, 1.0));
    }
    let reload = a.flag("reload");
    if reload || weather_changed {
        vp.load_weather(ed, reload).await;
    }
    if a.flag("rebake") {
        vp.with(|s| {
            s.game.rebake = true;
            s.game.roof = None;
        });
    }
    let want = a.bool("enabled").unwrap_or(vp.read(|s| s.game.enabled));
    let need = vp.with(|s| {
        let need = want && !s.game.weather_ok && !s.game.loading && !reload && !weather_changed;
        s.game.enabled = want;
        need
    });
    if need {
        vp.load_weather(ed, true).await;
    }
    if a.flag("weathers") && vp.read(|s| s.game.weathers.is_empty()) {
        vp.load_weather(ed, false).await;
    }
    // the answer names what the next frame draws
    if vp.read(|s| s.game.active())
        && let Some(rend) = vp.renderer_any(ed)
    {
        vp.settle().await;
        if let Some(p) = vp.make_packet(ed, &rend, false).await {
            rend.submit(p);
        }
    }
    redraw(ed);
    let mut out = game_state(&vp);
    if let Value::Object(m) = &mut out {
        if a.flag("weathers") {
            m.insert("weathers".into(), json!(vp.read(|s| s.game.weathers.clone())));
        }
        let everywhere = a.flag("all");
        if everywhere || a.flag("list") {
            // the lights drawn now, or with all=true every light of the level
            let list: Vec<Value> = if everywhere {
                match SceneService::of(ed) {
                    Ok(svc) => {
                        let snap = svc.store.snapshot();
                        let data = vp.game_data_for(ed).await;
                        let lights = ed.exec.cpu(move || lighting::collect(&snap, data.0.as_deref(), data.1.as_deref())).await;
                        lights.iter().map(light_json).collect()
                    }
                    Err(_) => Vec::new(),
                }
            } else {
                vp.read(|s| s.game.drawn.iter().map(light_json).collect())
            };
            m.insert("light_list".into(), Value::Array(list));
        }
    }
    let failed = vp.read(|s| s.game.enabled && !s.game.weather_ok);
    if failed {
        let text = vp.read(|s| s.game.error.clone());
        return Err(CmdError::new("game_lighting", text).with_data(out));
    }
    Ok(out)
}

/// The editor's frame clock: the idle cap is a preference, the rest is measured between asks.
fn frame_pacing(ed: &Arc<Editor>, a: &Args) -> Reply {
    let settings = gz_app::EditorSettings::of(ed);
    let requested = a.i64("active_idle_fps").unwrap_or(-1);
    let action = a.str("action").filter(|s| !s.is_empty()).unwrap_or(if requested >= 0 { "set" } else { "get" });
    if action.eq_ignore_ascii_case("set") {
        if !(30..=240).contains(&requested) {
            return Err(CmdError::invalid("active_idle_fps must be between 30 and 240"));
        }
        settings.set_pref("active_idle_fps", requested as f64);
        settings.save_soon(ed);
    } else if action.eq_ignore_ascii_case("reset") {
        settings.set_pref("active_idle_fps", 120.0);
        settings.save_soon(ed);
    } else if !action.eq_ignore_ascii_case("get") && !action.eq_ignore_ascii_case("stats") {
        return Err(CmdError::invalid("action must be get, stats, set, or reset"));
    }
    let stamp = ed.frames.current();
    let (fps, ms) = Viewport::of(ed).pacing(stamp.frame);
    let window = ed.frames.has_window();
    Ok(json!({
        "active_idle_fps": settings.pref("active_idle_fps").map_or(120, |v| v as u64),
        "background_poll_ms": 0,
        "measured_fps": (fps * 1000.0).round() / 1000.0,
        "measured_frame_ms": (ms * 1000.0).round() / 1000.0,
        // the editor draws on demand: a frame is explicit work, none is waiting
        "mode": if window { "explicit_work" } else { "background" },
        "last_wait_reason": "",
        "app_active": window,
        "play_in_editor": false,
        "realtime_render": false,
        "redraw_pending": false,
        "idle_cap_active": false,
        "frames": stamp.frame,
        "waits": 0,
        "waited_ms": 0.0,
    }))
}

async fn screenshot_viewport(ed: &Arc<Editor>) -> Reply {
    let vp = Viewport::of(ed);
    let image = vp.picture(ed).await.map_err(|e| CmdError::new(&e.code, "viewport RT unavailable").with_data(json!({ "detail": e.message })))?;
    let (w, h) = (image.width, image.height);
    let png = ed.exec.cpu(move || gz_app::encode_png(image.width, image.height, &image.rgba)).await?;
    let encoded = ed.exec.cpu(move || base64::engine::general_purpose::STANDARD.encode(png)).await;
    Ok(json!({ "png_base64": encoded, "width": w, "height": h }))
}
