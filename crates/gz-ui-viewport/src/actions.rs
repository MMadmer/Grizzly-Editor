//! The viewport's actions (the group "Viewport"): what its keys, its pills' rows and the palette
//! run. Each runs a command - `scene.mode` for the modes, `viewport.view` for the tool, the view
//! mode, what is shown and the snapping, `viewport.camera` for the camera, the scene's commands
//! for the selection - so every key can be bound again on the Keyboard page and every row is in
//! `help`. Their keys are live on the Level tab, and none of them while the camera flies (W, A,
//! S, D, Q and E are the camera's then).

use egui::Key;
use gz_app::{Action, ActionCx, Chord, Enabled, Scope, ShellBuilder};
use gz_editor::{Args, Value, json};
use gz_scene::{SceneMode, SceneService};
use gz_ui::Icon;

use crate::gizmo::{GizmoType, Space};
use crate::service::{SHOW_FLAGS, ViewMode, Viewport};

/// The tab the viewport's keys are live on.
pub const LEVEL: &[&str] = &["level"];
const TAB: Scope = Scope::Tab(LEVEL);

/// Esc while a mode other than Select is up. The level's Deselect All takes the key after it.
pub const LEAVE_MODE: &str = "viewport.leave_mode";
const GROUP: &str = "Viewport";

fn mode(cx: &ActionCx<'_>) -> Option<SceneMode> {
    cx.editor.project_opt()?.services.get::<SceneService>().map(|svc| svc.store.snapshot().tools.mode)
}

fn flying(cx: &ActionCx<'_>) -> bool {
    cx.editor.services.get::<Viewport>().is_some_and(|vp| vp.read(|s| s.nav.flying()))
}

/// On the Level tab with a scene, while the camera does not fly.
fn on_level(cx: &ActionCx<'_>) -> Enabled {
    if flying(cx) {
        return Enabled::Off("The camera is flying.");
    }
    Enabled::when(cx.kind == "level" && mode(cx).is_some(), "Open a scene first.")
}

fn on_selection(cx: &ActionCx<'_>) -> Enabled {
    if !on_level(cx).on() {
        return on_level(cx);
    }
    let any = cx
        .editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .is_some_and(|svc| !svc.store.snapshot().selection.is_empty());
    Enabled::when(any, "Select something first.")
}

/// The transform tools follow what every selected object can do.
fn can(cx: &ActionCx<'_>, i: usize) -> Enabled {
    if !on_level(cx).on() {
        return on_level(cx);
    }
    let sup = cx
        .editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map_or([true; 3], |svc| crate::gizmo::supports_of(&svc.store.snapshot()));
    Enabled::when(sup[i], "Something selected cannot take it.")
}

fn view_args(pairs: &[(&str, Value)]) -> Args {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

fn read<R>(cx: &ActionCx<'_>, f: impl FnOnce(&crate::service::ViewState) -> R) -> Option<R> {
    cx.editor.services.get::<Viewport>().map(|vp| vp.read(f))
}

/// An action of the viewport running `$cmd` with the arguments given.
macro_rules! act {
    ($id:literal, $title:literal, $cmd:literal, $icon:expr, $keys:expr, $on:expr, { $($k:literal : $v:tt),* $(,)? }) => {
        Action::new($id, $title, GROUP, $cmd)
            .icon($icon)
            .keys($keys)
            .scope(TAB)
            .args(|_| {
                let mut a = Args::new();
                $( a.insert($k.into(), json!($v)); )*
                a
            })
            .enabled($on)
    };
}

/// A switch of the Show pill: `viewport.view show=` or `hide=` its key.
macro_rules! show {
    ($id:literal, $title:literal, $key:literal) => {
        Action::new($id, $title, GROUP, "viewport.view")
            .scope(TAB)
            .args(|cx| {
                let on = read(cx, |s| {
                    SHOW_FLAGS.iter().find(|f| f.key == $key).is_some_and(|f| (f.get)(&s.prefs.show))
                })
                .unwrap_or(false);
                view_args(&[(if on { "hide" } else { "show" }, json!([$key]))])
            })
            .checked(|cx| {
                read(cx, |s| {
                    SHOW_FLAGS.iter().find(|f| f.key == $key).is_some_and(|f| (f.get)(&s.prefs.show))
                })
                .unwrap_or(false)
            })
    };
}

/// A snapping switch: `viewport.view <key>=` the other way.
macro_rules! snap {
    ($id:literal, $title:literal, $key:literal, $icon:expr, $get:expr) => {
        Action::new($id, $title, GROUP, "viewport.view")
            .icon($icon)
            .scope(TAB)
            .args(|cx| view_args(&[($key, json!(!read(cx, |s| $get(&s.prefs.snaps)).unwrap_or(false)))]))
            .checked(|cx| read(cx, |s| $get(&s.prefs.snaps)).unwrap_or(false))
    };
}

/// A view mode: `viewport.view mode=`.
macro_rules! view_mode {
    ($id:literal, $title:literal, $mode:expr, $icon:expr, $keys:expr) => {
        Action::new($id, $title, GROUP, "viewport.view")
            .icon($icon)
            .keys($keys)
            .scope(TAB)
            .args(|_| view_args(&[("mode", json!($mode.key()))]))
            .checked(|cx| read(cx, |s| s.prefs.mode == $mode).unwrap_or(false))
    };
}

/// A camera view: `viewport.camera action=set view=`.
macro_rules! camera_view {
    ($id:literal, $title:literal, $view:literal, $keys:expr) => {
        Action::new($id, $title, GROUP, "viewport.camera")
            .keys($keys)
            .scope(TAB)
            .args(|_| view_args(&[("action", json!("set")), ("view", json!($view))]))
            .checked(|cx| read(cx, |s| s.nav.current().key() == $view).unwrap_or(false))
    };
}

/// A bookmark's Go: on while the open scene's slot keeps a camera, so a stray digit passes on
/// instead of saying the slot is empty.
fn bookmark_kept(cx: &ActionCx<'_>, n: u8) -> Enabled {
    if !on_level(cx).on() {
        return on_level(cx);
    }
    Enabled::when(
        crate::service::bookmarks(cx.editor).contains(&n),
        "This camera bookmark keeps no camera yet: Ctrl and its digit keep one.",
    )
}

/// A camera bookmark: n goes to it, Ctrl+n keeps the camera in it.
macro_rules! bookmark {
    ($n:literal, $key:expr, $go:literal, $set:literal, $gt:literal, $st:literal) => {
        [
            act!($go, $gt, "viewport.camera", Icon::BookmarkNext, const { &[Chord::new($key)] }, |cx| bookmark_kept(cx, $n), {
                "action": "bookmark", "slot": $n
            }),
            act!($set, $st, "viewport.camera", Icon::Bookmark, const { &[Chord::ctrl($key)] }, on_level, {
                "action": "bookmark", "slot": $n, "store": true
            }),
        ]
    };
}

fn checked_mode(cx: &ActionCx<'_>, want: fn(SceneMode) -> bool) -> bool {
    mode(cx).is_some_and(want)
}

fn tool_is(cx: &ActionCx<'_>, g: GizmoType) -> bool {
    read(cx, |s| s.gizmo == g).unwrap_or(false)
}

/// Every action of the viewport.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn actions() -> Vec<Action> {
    let mut v = vec![
        // the modes
        act!("viewport.mode_select", "Select Mode", "scene.mode", Icon::Select, const { &[Chord::shift(Key::Num1)] }, on_level, { "mode": "select" })
            .checked(|cx| checked_mode(cx, |m| m == SceneMode::Select)),
        act!("viewport.mode_place", "Place Mode", "scene.mode", Icon::Place, const { &[Chord::shift(Key::Num2)] }, on_level, { "mode": "place" })
            .checked(|cx| checked_mode(cx, |m| m == SceneMode::Place)),
        Action::new("viewport.mode_paint", "Paint Mode", GROUP, "scene.mode")
            .icon(Icon::DetailObjects)
            .keys(const { &[Chord::shift(Key::Num3)] })
            .scope(TAB)
            .args(|cx| {
                let layer = read(cx, |s| s.paint_layer).unwrap_or(SceneMode::Details);
                view_args(&[("mode", json!(layer.word()))])
            })
            .enabled(on_level)
            .checked(|cx| checked_mode(cx, |m| m.layer().is_some())),
        act!("viewport.mode_zones", "Zones Mode", "scene.mode", Icon::Sector, const { &[Chord::shift(Key::Num4)] }, on_level, { "mode": "zones" })
            .checked(|cx| checked_mode(cx, |m| m == SceneMode::Zones)),
        Action::new(LEAVE_MODE, "Back to Select Mode", GROUP, "scene.mode")
            .icon(Icon::Select)
            .keys(const { &[Chord::new(Key::Escape)] })
            .scope(TAB)
            .args(|_| view_args(&[("mode", json!("select"))]))
            .enabled(|cx| {
                if !on_level(cx).on() {
                    return on_level(cx);
                }
                Enabled::when(mode(cx).is_some_and(|m| m != SceneMode::Select), "Select mode is up.")
            }),
        // the transform tools
        act!("viewport.select_tool", "Select", "viewport.view", Icon::Select, const { &[Chord::new(Key::Q)] }, on_level, { "tool": "select" })
            .checked(|cx| tool_is(cx, GizmoType::None)),
        act!("viewport.move_tool", "Move", "viewport.view", Icon::Move, const { &[Chord::new(Key::W)] }, |cx| can(cx, 0), { "tool": "move" })
            .checked(|cx| tool_is(cx, GizmoType::Move)),
        act!("viewport.rotate_tool", "Rotate", "viewport.view", Icon::Rotate, const { &[Chord::new(Key::E)] }, |cx| can(cx, 1), { "tool": "rotate" })
            .checked(|cx| tool_is(cx, GizmoType::Rotate)),
        act!("viewport.scale_tool", "Scale", "viewport.view", Icon::Scale, const { &[Chord::new(Key::R)] }, |cx| can(cx, 2), { "tool": "scale" })
            .checked(|cx| tool_is(cx, GizmoType::Scale)),
        Action::new("viewport.cycle_tool", "Next Transform Tool", GROUP, "viewport.view")
            .keys(const { &[Chord::new(Key::Space)] })
            .scope(TAB)
            .args(|cx| {
                let next = match read(cx, |s| s.gizmo).unwrap_or(GizmoType::None) {
                    GizmoType::Move => "rotate",
                    GizmoType::Rotate => "scale",
                    _ => "move",
                };
                view_args(&[("tool", json!(next))])
            })
            .enabled(on_level),
        Action::new("viewport.toggle_space", "World / Local", GROUP, "viewport.view")
            .icon(Icon::Globe)
            .keys(const { &[Chord::ctrl(Key::Backtick)] })
            .scope(TAB)
            .args(|cx| {
                let local = read(cx, |s| s.prefs.space == Space::Local).unwrap_or(false);
                view_args(&[("space", json!(if local { "world" } else { "local" }))])
            })
            .enabled(on_level)
            .checked(|cx| read(cx, |s| s.prefs.space == Space::Local).unwrap_or(false)),
        // the camera
        act!("viewport.frame_selection", "Frame Selection", "viewport.camera", Icon::FrameSelection, const { &[Chord::new(Key::F)] }, on_selection, { "action": "frame", "scope": "selection" }),
        act!("viewport.frame_all", "Frame All", "viewport.camera", Icon::FrameAll, const { &[Chord::new(Key::Home)] }, on_level, { "action": "frame", "scope": "all" }),
        camera_view!("viewport.view_perspective", "Perspective", "perspective", const { &[Chord::alt(Key::G)] }),
        camera_view!("viewport.view_top", "Top", "top", const { &[Chord::alt(Key::J)] }),
        camera_view!("viewport.view_bottom", "Bottom", "bottom", const { &[Chord::alt_shift(Key::J)] }),
        camera_view!("viewport.view_front", "Front", "front", const { &[Chord::alt(Key::H)] }),
        camera_view!("viewport.view_back", "Back", "back", const { &[Chord::alt_shift(Key::H)] }),
        camera_view!("viewport.view_left", "Left", "left", const { &[Chord::alt(Key::K)] }),
        camera_view!("viewport.view_right", "Right", "right", const { &[Chord::alt_shift(Key::K)] }),
        Action::new("viewport.orthographic", "Orthographic", GROUP, "viewport.camera")
            .icon(Icon::Ortho)
            .scope(TAB)
            .args(|cx| {
                let ortho = read(cx, |s| s.nav.ortho).unwrap_or(false);
                view_args(&[("action", json!("set")), ("orthographic", json!(!ortho))])
            })
            .checked(|cx| read(cx, |s| s.nav.ortho).unwrap_or(false)),
        // the selection as the viewport handles it
        act!("viewport.drop_to_floor", "Drop to Floor", "scene.align", Icon::ChevronDown, const { &[Chord::new(Key::End)] }, on_selection, { "action": "drop" }),
        act!("viewport.hide_selected", "Hide Selected", "scene.set", Icon::EyeOff, const { &[Chord::new(Key::H)] }, on_selection, { "hidden": true }),
        Action::new("viewport.show_all", "Show All Hidden", GROUP, "scene.set")
            .icon(Icon::Eye)
            .keys(const { &[Chord::ctrl(Key::H)] })
            .scope(TAB)
            .args(|cx| {
                let hidden: Vec<String> = cx
                    .editor
                    .project_opt()
                    .and_then(|s| s.services.get::<SceneService>())
                    .map(|svc| {
                        let snap = svc.store.snapshot();
                        snap.objects.values().filter(|o| !o.flags.visible).map(|o| o.name.to_string()).collect()
                    })
                    .unwrap_or_default();
                view_args(&[("objects", json!(hidden)), ("hidden", json!(false))])
            })
            .enabled(on_level),
        act!("viewport.group", "Group", "scene.group", Icon::Group, const { &[Chord::ctrl(Key::G)] }, on_selection, { "action": "group" }),
        // the view modes
        view_mode!("viewport.mode_studio", "Studio", ViewMode::Studio, Icon::Lit, const { &[Chord::alt(Key::Num4)] }),
        view_mode!("viewport.mode_unlit", "Unlit", ViewMode::Unlit, Icon::Unlit, const { &[Chord::alt(Key::Num3)] }),
        view_mode!("viewport.mode_game", "Game Lit", ViewMode::Game, Icon::Lit, const { &[Chord::alt(Key::Num5)] }),
        view_mode!("viewport.mode_wireframe", "Wireframe", ViewMode::Wireframe, Icon::Wireframe, const { &[Chord::alt(Key::Num2)] }),
        act!("viewport.rebake_roofs", "Rebake Roof Map", "viewport.view", Icon::Refresh, &[], on_level, { "rebake": true }),
        // what is shown
        show!("viewport.show_grid", "Grid", "grid"),
        show!("viewport.show_safe_rect", "Safe Frame", "safe_rect"),
        show!("viewport.show_stats", "Stats", "stats"),
        show!("viewport.show_spawn_helpers", "Spawn Shapes", "spawn_helpers"),
        show!("viewport.show_spawn_names", "Spawn Names", "spawn_names"),
        show!("viewport.show_light_names", "Light Names", "light_names"),
        show!("viewport.show_glow_crosses", "Glow Crosses", "glow_crosses"),
        show!("viewport.show_ai_map", "AI Map", "ai_map"),
        show!("viewport.show_fog", "Fog", "fog"),
        // the snapping
        snap!("viewport.grid_snap", "Grid Snap", "grid_snap", Icon::SnapGrid, |s: &crate::service::Snaps| s.grid),
        snap!("viewport.angle_snap", "Angle Snap", "angle_snap", Icon::SnapAngle, |s: &crate::service::Snaps| s.angle),
        snap!("viewport.scale_snap", "Scale Snap", "scale_snap", Icon::SnapScale, |s: &crate::service::Snaps| s.scale),
        snap!("viewport.surface_snap", "Surface Snap", "surface_snap", Icon::SnapObject, |s: &crate::service::Snaps| s.surface),
        snap!("viewport.surface_align", "Align to Surface", "surface_align", Icon::None, |s: &crate::service::Snaps| s.align),
        snap!("viewport.vertex_snap", "Snap to Vertices", "vertex_snap", Icon::None, |s: &crate::service::Snaps| s.vertex),
    ];
    for pair in [
        bookmark!(
            1,
            Key::Num1,
            "viewport.bookmark_1",
            "viewport.set_bookmark_1",
            "Go to Camera Bookmark 1",
            "Set Camera Bookmark 1"
        ),
        bookmark!(
            2,
            Key::Num2,
            "viewport.bookmark_2",
            "viewport.set_bookmark_2",
            "Go to Camera Bookmark 2",
            "Set Camera Bookmark 2"
        ),
        bookmark!(
            3,
            Key::Num3,
            "viewport.bookmark_3",
            "viewport.set_bookmark_3",
            "Go to Camera Bookmark 3",
            "Set Camera Bookmark 3"
        ),
        bookmark!(
            4,
            Key::Num4,
            "viewport.bookmark_4",
            "viewport.set_bookmark_4",
            "Go to Camera Bookmark 4",
            "Set Camera Bookmark 4"
        ),
        bookmark!(
            5,
            Key::Num5,
            "viewport.bookmark_5",
            "viewport.set_bookmark_5",
            "Go to Camera Bookmark 5",
            "Set Camera Bookmark 5"
        ),
        bookmark!(
            6,
            Key::Num6,
            "viewport.bookmark_6",
            "viewport.set_bookmark_6",
            "Go to Camera Bookmark 6",
            "Set Camera Bookmark 6"
        ),
        bookmark!(
            7,
            Key::Num7,
            "viewport.bookmark_7",
            "viewport.set_bookmark_7",
            "Go to Camera Bookmark 7",
            "Set Camera Bookmark 7"
        ),
        bookmark!(
            8,
            Key::Num8,
            "viewport.bookmark_8",
            "viewport.set_bookmark_8",
            "Go to Camera Bookmark 8",
            "Set Camera Bookmark 8"
        ),
        bookmark!(
            9,
            Key::Num9,
            "viewport.bookmark_9",
            "viewport.set_bookmark_9",
            "Go to Camera Bookmark 9",
            "Set Camera Bookmark 9"
        ),
    ] {
        v.extend(pair);
    }
    v
}

/// The modes' actions in the mode control's order.
pub const MODES: [&str; 4] =
    ["viewport.mode_select", "viewport.mode_place", "viewport.mode_paint", "viewport.mode_zones"];

/// The viewport's words in Russian.
pub const RU: &str = include_str!("ru.lua");

/// Adds the viewport's actions, its preferences and its words to the shell.
pub fn install(b: &mut ShellBuilder) {
    for a in actions() {
        b.action(a);
    }
    crate::prefs::install(b);
    b.translation(RU);
}

#[cfg(test)]
mod tests {
    use super::actions;

    /// Every action lives in the group Viewport on the Level tab, no two share an id, and no two
    /// take the same key.
    #[test]
    fn every_action_has_its_own_id_and_keys() {
        let all = actions();
        for (i, a) in all.iter().enumerate() {
            assert_eq!(a.group, "Viewport");
            assert!(a.id.starts_with("viewport."), "{}", a.id);
            for b in all.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
                assert!(a.keys.iter().all(|k| !b.keys.contains(k)), "{} and {} share a key", a.id, b.id);
            }
        }
        for m in super::MODES {
            assert!(all.iter().any(|a| a.id == m), "{m}");
        }
    }

    /// The viewport's catalog reads with the kit's and the shell's, and holds every title, group
    /// and reason of its actions.
    #[test]
    fn the_catalog_is_sound_and_names_every_action() {
        let c = gz_ui::i18n::Catalog::parse(&[gz_ui::i18n::KIT_RU, gz_app::lang::SHELL_RU, super::RU])
            .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
        for a in actions() {
            assert!(c.knows(a.title), "no entry for the action title '{}'", a.title);
            assert!(c.knows(a.group), "no entry for the group '{}'", a.group);
        }
    }
}
