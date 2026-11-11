//! The viewport's menus (`UIViewportToolbar`, `DrawViewportOptions`, `GameLighting_Menu`):
//! what the pills open and what the Level tab's View menu holds. Every item with a command
//! shows the keys bound to it now.

use std::sync::Arc;

use egui::Ui;
use gz_app::DocCx;
use gz_app::chrome::local_submenu;
use gz_editor::Editor;
use gz_ui::{Icon, MenuItem, menu_item};

use crate::camera::Target;
use crate::gizmo::GizmoType;
use crate::keys::{self, Act};
use crate::lighting::hour_text;
use crate::service::{Fill, Viewport};

/// The steps the snaps have always offered.
pub const MOVE_STEPS: [f32; 9] = [0.01, 0.05, 0.1, 1.0, 2.0, 5.0, 50.0, 200.0, 1000.0];
pub const SCALE_STEPS: [f32; 9] = [0.01, 0.05, 0.1, 1.0, 2.0, 5.0, 50.0, 200.0, 1000.0];
pub const ANGLE_STEPS: [f32; 7] = [1.406_25, 2.8125, 5.625, 11.25, 22.5, 45.0, 90.0];

/// `%g`: 5.625 reads 5.625, 0.1 reads 0.1, never 5.62500.
#[must_use]
pub fn fmt_g(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('e') { format!("{v:.6}").trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

fn check(ui: &mut Ui, label: &str, keys: &str, on: bool) -> bool {
    menu_item(ui, label, MenuItem { keys, selected: on, ..MenuItem::default() })
}

/// The View menu's viewport options (`DrawViewportOptions`), for the Level tab's View menu;
/// Maximize Viewport is the Level tab's own.
pub fn view_menu(ui: &mut Ui, cx: &mut DocCx<'_>) {
    options_body(ui, cx.editor);
}

/// View Options (`DrawViewportOptions`): the rarely changed render switches folded under
/// Render, then the safe rect and the grid, fog and environment, the lighting, the sounds and
/// Real Time, the helpers and stats.
pub(crate) fn options_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let v = vp.read(|s| s.view.clone());
    local_submenu(ui, "Render", Icon::None, true, |ui| render_body(ui, editor));
    ui.separator();
    if check(ui, "Safe Rect", &keys::keys_of(editor, Act::SafeRect), v.safe_rect) {
        keys::run(editor, Act::SafeRect);
    }
    if check(ui, "Grid", &keys::keys_of(editor, Act::Grid), v.grid) {
        keys::run(editor, Act::Grid);
    }
    ui.separator();
    if check(ui, "Fog", "", v.fog) {
        vp.with(|s| s.view.fog = !s.view.fog);
    }
    if check(ui, "Environment", "", v.environment) {
        vp.with(|s| s.view.environment = !s.view.environment);
    }
    ui.separator();
    if check(ui, "Lighting", "", v.lighting) {
        vp.with(|s| s.view.lighting = !s.view.lighting);
    }
    local_submenu(ui, "Game Lighting", Icon::None, true, |ui| game_body(ui, editor));
    let settings = gz_app::EditorSettings::of(editor);
    let (mute, real) = (settings.render_flag("mute_sounds"), crate::service::real_time(editor));
    if check(ui, "Mute Sounds", "", mute) {
        crate::service::set_mute_sounds(editor, !mute);
    }
    if check(ui, "Real Time", "", real) {
        crate::service::set_real_time(editor, !real);
    }
    ui.separator();
    if check(ui, "Spawn Helpers", "", v.helpers) {
        vp.with(|s| s.view.helpers = !s.view.helpers);
    }
    if check(ui, "Stats", "", v.stats) {
        vp.with(|s| s.view.stats = !s.view.stats);
    }
}

/// View > Render: GPU, Quality, Fill Mode, Shade Mode, Edged Faces, Linear Filtering, Textures.
fn render_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let v = vp.read(|s| s.view.clone());
    local_submenu(ui, "GPU", Icon::None, true, |ui| gpu_body(ui, editor));
    local_submenu(ui, "Quality", Icon::None, true, |ui| {
        for (q, label) in crate::service::QUALITIES {
            if check(ui, label, "", (v.quality - q).abs() < 0.01) {
                vp.set_quality(q);
            }
        }
    });
    local_submenu(ui, "Fill Mode", Icon::None, true, |ui| {
        let wire_keys = keys::keys_of(editor, Act::Wireframe);
        for (fill, label, k) in [
            (Fill::Point, "Point", ""),
            (Fill::Wireframe, "Wireframe", wire_keys.as_str()),
            (Fill::Solid, "Solid", ""),
        ] {
            if check(ui, label, k, v.fill == fill) {
                vp.with(|s| s.view.fill = fill);
            }
        }
    });
    local_submenu(ui, "Shade Mode", Icon::None, true, |ui| {
        if check(ui, "Flat", "", v.flat) {
            vp.with(|s| s.view.flat = true);
        }
        if check(ui, "Gouraud", "", !v.flat) {
            vp.with(|s| s.view.flat = false);
        }
    });
    if check(ui, "Edged Faces", "", v.edged_faces) {
        vp.with(|s| s.view.edged_faces = !s.view.edged_faces);
    }
    ui.separator();
    if check(ui, "Linear Filtering", "", v.filter_linear) {
        vp.with(|s| s.view.filter_linear = !s.view.filter_linear);
    }
    if check(ui, "Textures", "", v.textures) {
        vp.with(|s| s.view.textures = !s.view.textures);
    }
}

/// View > Render > GPU: the adapters by name and the driver's hint; both take a restart.
fn gpu_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let gpu = gz_app::gpu::Gpu::of(editor);
    let settings = gz_app::EditorSettings::of(editor);
    let adapters = gpu.adapters().unwrap_or_default();
    let active = if adapters.active.is_empty() { "unknown" } else { adapters.active.as_str() };
    menu_item(ui, &format!("Active: {active}"), MenuItem { enabled: false, ..MenuItem::default() });
    ui.separator();
    let wanted = settings.gpu_adapter();
    let pick = |name: &str| {
        settings.set_gpu_adapter(name);
        settings.save_soon(editor);
    };
    if check(ui, "System Default", "", wanted.is_empty()) {
        pick("");
    }
    for name in &adapters.names {
        if check(ui, name, "", *name == wanted) {
            pick(name);
        }
    }
    if adapters.names.is_empty() {
        menu_item(ui, "No adapters reported", MenuItem { enabled: false, ..MenuItem::default() });
    }
    ui.separator();
    let hint = gpu.hint(editor);
    for h in gz_app::gpu::DriverHint::ALL {
        if menu_item(
            ui,
            h.title(),
            MenuItem { selected: h == hint, enabled: gpu.hint_known(), ..MenuItem::default() },
        ) {
            gpu.set_hint(editor, h);
        }
    }
    ui.separator();
    menu_item(ui, "Takes effect after a restart", MenuItem { enabled: false, ..MenuItem::default() });
}

/// Game Lighting (`GameLighting_Menu`).
pub(crate) fn game_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let g = vp.read(|s| s.game.clone());
    if check(ui, "Enabled", "", g.enabled) {
        set_game_enabled(editor, !g.enabled);
    }
    if g.enabled && !g.weather_ok && !g.error.is_empty() {
        menu_item(ui, &g.error, MenuItem { enabled: false, ..MenuItem::default() });
    }
    ui.separator();
    local_submenu(ui, "Weather", Icon::None, true, |ui| {
        // read as the submenu opens: the linked game may have changed
        if g.weathers.is_empty() && !g.loading {
            reload(editor, false);
        }
        if menu_item(ui, "Default (a new game's first night)", MenuItem::default()) {
            set_weather(editor, Some(String::new()), None);
        }
        ui.separator();
        for name in &g.weathers {
            if check(ui, name, "", *name == g.weather) {
                set_weather(editor, Some(name.clone()), None);
            }
        }
    });
    local_submenu(ui, "Hour", Icon::None, true, |ui| {
        for i in 0..24 {
            // the evening first: the night is what this view is for
            let h = ((i + 18) % 24) as f32;
            if check(ui, &hour_text(h), "", (g.hour - h).abs() < 1e-3) {
                set_weather(editor, None, Some(h));
            }
        }
    });
    local_submenu(ui, "Exposure", Icon::None, true, |ui| {
        if check(ui, "Game Tonemap", "", g.exposure <= 0.0) {
            vp.with(|s| s.game.exposure = 0.0);
        }
        for e in [1.0f32, 1.5, 2.0, 3.0, 4.0] {
            if check(ui, &format!("x{e:.1}"), "", (g.exposure - e).abs() < 1e-3) {
                vp.with(|s| s.game.exposure = e);
            }
        }
    });
    local_submenu(ui, "Indoor Sky Light", Icon::None, true, |ui| {
        for share in [0.0f32, 0.05, 0.1, 0.15, 0.25, 0.5, 1.0] {
            let label = format!("{}%", (share * 100.0 + 0.5).floor() as i32);
            if check(ui, &label, "", (g.indoor_hemi - share).abs() < 1e-3) {
                vp.with(|s| s.game.indoor_hemi = share);
            }
        }
    });
    ui.separator();
    if menu_item(ui, "Rebake Roof Map", MenuItem { enabled: g.enabled, ..MenuItem::default() }) {
        vp.with(|s| {
            s.game.rebake = true;
            s.game.roof = None;
        });
    }
    if g.enabled && g.weather_ok {
        let info = format!("{} lights, {} at {}", g.drawn.len(), g.weather, hour_text(g.hour));
        menu_item(ui, &info, MenuItem { enabled: false, ..MenuItem::default() });
    }
}

/// `Enable` / `Disable`: on, the weather is read when it has not been.
pub(crate) fn set_game_enabled(editor: &Arc<Editor>, on: bool) {
    let vp = Viewport::of(editor);
    let need = vp.with(|s| {
        s.game.enabled = on;
        on && !s.game.weather_ok && !s.game.loading
    });
    if need {
        reload(editor, true);
    }
}

/// `SetWeather(name, hour)`: read off the UI thread.
pub(crate) fn set_weather(editor: &Arc<Editor>, weather: Option<String>, hour: Option<f32>) {
    let vp = Viewport::of(editor);
    vp.with(|s| {
        if let Some(w) = weather {
            s.game.weather = w;
        }
        if let Some(h) = hour {
            s.game.hour = h;
        }
    });
    reload(editor, false);
}

fn reload(editor: &Arc<Editor>, settings: bool) {
    let vp = Viewport::of(editor);
    vp.with(|s| s.game.loading = true);
    let ed = editor.clone();
    editor.exec.spawn(async move { vp.load_weather(&ed, settings).await });
}

/// The Perspective pill: the free view, the six axis views, the projection, Frame All and
/// Frame Selection.
pub(crate) fn perspective_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let (current, ortho) = vp.read(|s| (s.nav.current(), s.nav.ortho));
    if check(ui, Target::Perspective.name(), "", current == Target::Perspective) {
        vp.with(|s| s.nav.set_target(Target::Perspective));
    }
    ui.separator();
    for t in Target::AXES {
        if check(ui, t.name(), "", current == t) {
            vp.with(|s| s.nav.set_target(t));
        }
    }
    ui.separator();
    if check(ui, "Orthographic", "", ortho) {
        vp.with(|s| s.nav.ortho = !s.nav.ortho);
    }
    ui.separator();
    if menu_item(
        ui,
        "Frame All",
        MenuItem { icon: Icon::FrameAll, keys: &keys::keys_of(editor, Act::FrameAll), ..MenuItem::default() },
    ) {
        keys::run(editor, Act::FrameAll);
    }
    if menu_item(
        ui,
        "Frame Selection",
        MenuItem {
            icon: Icon::FrameSelection,
            keys: &keys::keys_of(editor, Act::FrameSelection),
            ..MenuItem::default()
        },
    ) {
        keys::run(editor, Act::FrameSelection);
    }
}

/// The Show pill: Stats, Grid, Safe Rect.
pub(crate) fn show_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let v = vp.read(|s| s.view.clone());
    if check(ui, "Stats", "", v.stats) {
        vp.with(|s| s.view.stats = !s.view.stats);
    }
    if check(ui, "Grid", &keys::keys_of(editor, Act::Grid), v.grid) {
        keys::run(editor, Act::Grid);
    }
    if check(ui, "Safe Rect", &keys::keys_of(editor, Act::SafeRect), v.safe_rect) {
        keys::run(editor, Act::SafeRect);
    }
}

/// The snap options: object snap, vertex snap, normal alignment.
pub(crate) fn snap_body(ui: &mut Ui, editor: &Arc<Editor>) {
    let vp = Viewport::of(editor);
    let st = vp.read(|s| s.settings);
    if check(ui, "Snap to Objects", "", st.object_snap) {
        vp.with(|s| s.settings.object_snap = !s.settings.object_snap);
    }
    if check(ui, "Snap to Vertices", "", st.vertex_snap) {
        vp.with(|s| s.settings.vertex_snap = !s.settings.vertex_snap);
    }
    if check(ui, "Align to Surface Normal", "", st.normal_align) {
        vp.with(|s| s.settings.normal_align = !s.settings.normal_align);
    }
}

/// The label of a snap's step: `%g`, degrees for the angle.
#[must_use]
pub fn step_label(kind: GizmoType, v: f32) -> String {
    if kind == GizmoType::Rotate { format!("{}\u{b0}", fmt_g(v)) } else { fmt_g(v) }
}

/// A snap's steps.
pub(crate) fn step_body(ui: &mut Ui, editor: &Arc<Editor>, kind: GizmoType) {
    let vp = Viewport::of(editor);
    let current = vp.read(|s| s.step(kind));
    let steps: &[f32] = match kind {
        GizmoType::Move => &MOVE_STEPS,
        GizmoType::Scale => &SCALE_STEPS,
        _ => &ANGLE_STEPS,
    };
    for v in steps {
        if check(ui, &step_label(kind, *v), "", (v - current).abs() < v * 1e-4) {
            set_step(editor, kind, *v);
        }
    }
}

/// `Gizmo::SetStep`: the step is the preference, saved.
pub(crate) fn set_step(editor: &Arc<Editor>, kind: GizmoType, v: f32) {
    let key = match kind {
        GizmoType::Move => "snap_grid_step",
        GizmoType::Scale => "snap_scale_step",
        GizmoType::Rotate => "snap_angle_step",
        GizmoType::None => return,
    };
    let settings = gz_app::EditorSettings::of(editor);
    settings.set_pref(key, f64::from(v));
    settings.save_soon(editor);
    Viewport::of(editor).sync(editor);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_like_printf_g() {
        assert_eq!(fmt_g(5.625), "5.625");
        assert_eq!(fmt_g(0.1), "0.1");
        assert_eq!(fmt_g(1000.0), "1000");
        assert_eq!(step_label(GizmoType::Rotate, 45.0), "45\u{b0}");
    }
}
