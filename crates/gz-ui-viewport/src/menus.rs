//! What the pills open and what the Level tab's View menu holds. Every row with a name is the
//! viewport's action (`actions.rs`), drawn by `gz_app::actions::menu_row` with the keys bound to
//! it now; the lists of values (a snap's steps, the weathers, the hours) set them through
//! `viewport.view`.

use std::sync::Arc;

use egui::Ui;
use gz_app::DocCx;
use gz_app::actions::menu_row;
use gz_app::chrome::local_submenu;
use gz_editor::{Args, Editor, Value, json};
use gz_ui::{Icon, MenuItem, Tx, menu_item, tr};

use crate::gizmo::GizmoType;
use crate::lighting::hour_text;
use crate::service::{ANGLE_STEPS, MOVE_STEPS, SCALE_STEPS, ViewMode, Viewport};

/// `%g`: 22.5 reads 22.5, 0.1 reads 0.1, never 0.100000.
#[must_use]
pub fn fmt_g(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('e') { format!("{v:.6}").trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

/// A snap's step with its unit, in the window's language: "0.1 m", "15°", "x0.25".
#[must_use]
pub fn step_text(kind: GizmoType, v: f32) -> Tx<'static> {
    match kind {
        GizmoType::Rotate => Tx::raw(format!("{}\u{b0}", fmt_g(v))),
        GizmoType::Scale => Tx::raw(format!("x{}", fmt_g(v))),
        _ => tr!("{v} m", v = fmt_g(v)),
    }
}

/// Sets values of the view through `viewport.view`, as an author's edit.
fn set_view(editor: &Arc<Editor>, pairs: &[(&str, Value)]) {
    let a: Args = pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
    editor.post("viewport.view", a);
}

fn check<'a>(ui: &mut Ui, label: impl Into<Tx<'a>>, on: bool) -> bool {
    menu_item(ui, label, MenuItem { selected: on, ..MenuItem::default() })
}

/// The Level tab's View menu: the view modes and the framing. What is shown is the Show pill's
/// alone.
pub fn view_menu(ui: &mut Ui, cx: &mut DocCx<'_>) {
    view_mode_body(ui, cx);
    ui.separator();
    for id in ["viewport.frame_selection", "viewport.frame_all", "viewport.toggle_space"] {
        menu_row(ui, cx, id, None);
    }
}

/// The View Mode pill: Studio, Unlit, Game Lit (its weather and hour while it is up), Wireframe.
pub(crate) fn view_mode_body(ui: &mut Ui, cx: &mut DocCx<'_>) {
    for id in ["viewport.mode_studio", "viewport.mode_unlit", "viewport.mode_game", "viewport.mode_wireframe"]
    {
        menu_row(ui, cx, id, None);
    }
    let editor = cx.editor.clone();
    let game = Viewport::of(&editor).read(|s| s.prefs.mode == ViewMode::Game);
    if game {
        ui.separator();
        game_body(ui, cx);
    }
}

/// Game Lit's weather, hour, exposure and indoor sky light.
fn game_body(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    let vp = Viewport::of(&editor);
    let g = vp.read(|s| s.game.clone());
    if !g.weather_ok && !g.error.is_empty() {
        menu_item(ui, Tx::ui(&g.error), MenuItem { enabled: false, ..MenuItem::default() });
    }
    local_submenu(ui, "Weather", Icon::None, true, |ui| {
        if g.weathers.is_empty() && !g.loading {
            set_view(&editor, &[("weathers", json!(true))]);
        }
        if check(ui, "Default (a new game's first night)", g.weather.is_empty()) {
            set_view(&editor, &[("weather", json!(""))]);
        }
        ui.separator();
        for name in &g.weathers {
            if check(ui, Tx::raw(name), *name == g.weather) {
                set_view(&editor, &[("weather", json!(name))]);
            }
        }
    });
    local_submenu(ui, "Hour", Icon::None, true, |ui| {
        for i in 0..24 {
            // the evening first: the night is what this view is for
            let h = ((i + 18) % 24) as f32;
            if check(ui, Tx::raw(hour_text(h)), (g.hour - h).abs() < 1e-3) {
                set_view(&editor, &[("hour", json!(h))]);
            }
        }
    });
    local_submenu(ui, "Exposure", Icon::None, true, |ui| {
        if check(ui, "Game Tonemap", g.exposure <= 0.0) {
            set_view(&editor, &[("exposure", json!(0.0))]);
        }
        for e in [1.0f32, 1.5, 2.0, 3.0, 4.0] {
            if check(ui, Tx::raw(format!("x{e:.1}")), (g.exposure - e).abs() < 1e-3) {
                set_view(&editor, &[("exposure", json!(e))]);
            }
        }
    });
    local_submenu(ui, "Indoor Sky Light", Icon::None, true, |ui| {
        for share in [0.0f32, 0.05, 0.1, 0.15, 0.25, 0.5, 1.0] {
            let label = format!("{} %", (share * 100.0 + 0.5).floor() as i32);
            if check(ui, Tx::raw(label), (g.indoor_hemi - share).abs() < 1e-3) {
                set_view(&editor, &[("indoor_hemi", json!(share))]);
            }
        }
    });
    menu_row(ui, cx, "viewport.rebake_roofs", None);
    if g.weather_ok {
        let info = tr!(
            "{n} {n:light|lights}, {weather} at {hour}",
            n = g.drawn.len(),
            weather = g.weather.as_str(),
            hour = hour_text(g.hour)
        );
        menu_item(ui, info, MenuItem { enabled: false, ..MenuItem::default() });
    }
}

/// The rows of the camera bookmarks that hold a view, 1 to 9.
const GO_TO_BOOKMARK: [&str; 9] = [
    "Go to Bookmark 1",
    "Go to Bookmark 2",
    "Go to Bookmark 3",
    "Go to Bookmark 4",
    "Go to Bookmark 5",
    "Go to Bookmark 6",
    "Go to Bookmark 7",
    "Go to Bookmark 8",
    "Go to Bookmark 9",
];

/// The Perspective pill: the free view, the six axis views, the projection, the framing, the
/// camera bookmarks.
pub(crate) fn perspective_body(ui: &mut Ui, cx: &mut DocCx<'_>) {
    menu_row(ui, cx, "viewport.view_perspective", None);
    ui.separator();
    for id in [
        "viewport.view_top",
        "viewport.view_bottom",
        "viewport.view_front",
        "viewport.view_back",
        "viewport.view_left",
        "viewport.view_right",
    ] {
        menu_row(ui, cx, id, None);
    }
    ui.separator();
    menu_row(ui, cx, "viewport.orthographic", None);
    ui.separator();
    menu_row(ui, cx, "viewport.frame_selection", None);
    menu_row(ui, cx, "viewport.frame_all", None);
    let set = crate::service::bookmarks(cx.editor);
    local_submenu(ui, "Camera Bookmarks", Icon::Bookmark, true, |ui| {
        for (n, go) in (1..=9u8).zip(GO_TO_BOOKMARK) {
            if set.contains(&n) {
                menu_row(ui, cx, &format!("viewport.bookmark_{n}"), Some(go));
            } else {
                let empty = tr!("Bookmark {n} (empty)", n = n);
                menu_item(ui, empty, MenuItem { enabled: false, ..MenuItem::default() });
            }
        }
        ui.separator();
        for n in 1..=9u8 {
            menu_row(ui, cx, &format!("viewport.set_bookmark_{n}"), None);
        }
    });
    ui.separator();
    // the Level tab's own action
    menu_row(ui, cx, "level.maximize_viewport", None);
}

/// The Show pill: the overlays, the helpers of each kind, the fog.
pub(crate) fn show_body(ui: &mut Ui, cx: &mut DocCx<'_>) {
    for (i, id) in [
        "viewport.show_grid",
        "viewport.show_safe_rect",
        "viewport.show_stats",
        "",
        "viewport.show_spawn_helpers",
        "viewport.show_spawn_names",
        "viewport.show_light_names",
        "viewport.show_glow_crosses",
        "viewport.show_ai_map",
        "",
        "viewport.show_fog",
    ]
    .into_iter()
    .enumerate()
    {
        if id.is_empty() {
            if i > 0 {
                ui.separator();
            }
            continue;
        }
        menu_row(ui, cx, id, None);
    }
}

/// Surface Snap's options.
pub(crate) fn surface_body(ui: &mut Ui, cx: &mut DocCx<'_>) {
    for id in ["viewport.surface_snap", "viewport.surface_align", "viewport.vertex_snap"] {
        menu_row(ui, cx, id, None);
    }
}

/// A snap's steps.
pub(crate) fn step_body(ui: &mut Ui, cx: &mut DocCx<'_>, kind: GizmoType) {
    let editor = cx.editor.clone();
    let current = Viewport::of(&editor).read(|s| s.step(kind));
    let (steps, key): (&[f32], &str) = match kind {
        GizmoType::Move => (&MOVE_STEPS, "grid_step"),
        GizmoType::Scale => (&SCALE_STEPS, "scale_step"),
        _ => (&ANGLE_STEPS, "angle_step"),
    };
    for v in steps {
        if check(ui, step_text(kind, *v), (v - current).abs() < v * 1e-4) {
            set_view(&editor, &[(key, json!(v))]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_with_their_units() {
        assert_eq!(fmt_g(22.5), "22.5");
        assert_eq!(fmt_g(0.1), "0.1");
        assert_eq!(step_text(GizmoType::Rotate, 45.0).en(), "45\u{b0}");
        assert_eq!(step_text(GizmoType::Move, 0.25).en(), "0.25 m");
        assert_eq!(step_text(GizmoType::Scale, 0.5).en(), "x0.5");
    }
}
