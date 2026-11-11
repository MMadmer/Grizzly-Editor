//! The scene's key bindings and the viewport's commands: one table, the C++ ids and names
//! (`kCommandNames`), the default keys the editor shipped with, and what each one does. The
//! keys bound NOW are the default unless the author bound another in the settings.

use std::sync::Arc;

use egui::Key;
use gz_app::{Chord, DocCx, ShellBuilder, UiCommand};
use gz_editor::{Args, Editor};
use gz_scene::apply::SelectMode;
use gz_scene::{ObjClass, SceneCmd, SceneService};
use gz_ui::Icon;

use crate::gizmo::GizmoType;
use crate::service::{Action, Fill, Viewport};

/// What a binding does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Gizmo(GizmoType),
    CycleGizmo,
    Place,
    StopPlacing,
    FrameAll,
    FrameSelection,
    Wireframe,
    Grid,
    GridSmaller,
    GridLarger,
    SafeRect,
    DropToGround,
    Delete,
    SelectAll,
    DeselectAll,
    InvertSelection,
    HideSelected,
    HideUnselected,
}

/// A command of the table: its `.ini` id, group, name, icon, default keys and what it does.
pub struct Binding {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub icon: Icon,
    pub keys: Option<Chord>,
    pub act: Act,
    /// The viewport owns it: it goes on the Keyboard page from [`install`]. The others are the
    /// scene's, registered by the Level tab, and only answered here when routed.
    pub own: bool,
}

const fn c(key: Key) -> Option<Chord> {
    Some(Chord::new(key))
}

const fn ctrl(key: Key) -> Option<Chord> {
    Some(Chord::ctrl(key))
}

/// The table, in the Keyboard page's order.
pub const BINDINGS: &[Binding] = &[
    Binding {
        id: "COMMAND_GIZMO_MODE.\"Select\"",
        group: "Viewport Tools",
        label: "Select",
        icon: Icon::Select,
        keys: c(Key::Q),
        act: Act::Gizmo(GizmoType::None),
        own: true,
    },
    Binding {
        id: "COMMAND_CHANGE_ACTION.\"Add\"",
        group: "Viewport Tools",
        label: "Place",
        icon: Icon::Place,
        keys: c(Key::A),
        act: Act::Place,
        own: true,
    },
    Binding {
        id: "COMMAND_GIZMO_MODE.\"Move\"",
        group: "Viewport Tools",
        label: "Move",
        icon: Icon::Move,
        keys: c(Key::W),
        act: Act::Gizmo(GizmoType::Move),
        own: true,
    },
    Binding {
        id: "COMMAND_GIZMO_MODE.\"Rotate\"",
        group: "Viewport Tools",
        label: "Rotate",
        icon: Icon::Rotate,
        keys: c(Key::E),
        act: Act::Gizmo(GizmoType::Rotate),
        own: true,
    },
    Binding {
        id: "COMMAND_GIZMO_MODE.\"Scale\"",
        group: "Viewport Tools",
        label: "Scale",
        icon: Icon::Scale,
        keys: c(Key::R),
        act: Act::Gizmo(GizmoType::Scale),
        own: true,
    },
    Binding {
        id: "COMMAND_GIZMO_MODE.\"Cycle\"",
        group: "Viewport Tools",
        label: "Next Transform Tool",
        icon: Icon::None,
        keys: c(Key::Space),
        act: Act::CycleGizmo,
        own: true,
    },
    Binding {
        id: "COMMAND_CHANGE_ACTION.\"Select\"",
        group: "Viewport Tools",
        label: "Stop Placing",
        icon: Icon::None,
        keys: c(Key::S),
        act: Act::StopPlacing,
        own: true,
    },
    Binding {
        id: "COMMAND_ZOOM_EXTENTS.\"Extent\"",
        group: "View",
        label: "Frame All",
        icon: Icon::FrameAll,
        keys: None,
        act: Act::FrameAll,
        own: true,
    },
    Binding {
        id: "COMMAND_ZOOM_EXTENTS.\"Selected\"",
        group: "View",
        label: "Frame Selection",
        icon: Icon::FrameSelection,
        keys: c(Key::F),
        act: Act::FrameSelection,
        own: true,
    },
    Binding {
        id: "COMMAND_TOGGLE_RENDER_WIRE",
        group: "View",
        label: "Wireframe",
        icon: Icon::Wireframe,
        keys: c(Key::F3),
        act: Act::Wireframe,
        own: true,
    },
    Binding {
        id: "COMMAND_TOGGLE_GRID",
        group: "View",
        label: "Show Grid",
        icon: Icon::None,
        keys: ctrl(Key::G),
        act: Act::Grid,
        own: true,
    },
    Binding {
        id: "COMMAND_GRID_SLOT_SIZE.\"Decrease\"",
        group: "View",
        label: "Smaller Grid Cells",
        icon: Icon::None,
        keys: c(Key::OpenBracket),
        act: Act::GridSmaller,
        own: true,
    },
    Binding {
        id: "COMMAND_GRID_SLOT_SIZE.\"Increase\"",
        group: "View",
        label: "Larger Grid Cells",
        icon: Icon::None,
        keys: c(Key::CloseBracket),
        act: Act::GridLarger,
        own: true,
    },
    Binding {
        id: "COMMAND_TOGGLE_SAFE_RECT",
        group: "View",
        label: "Show Safe Rect",
        icon: Icon::None,
        keys: ctrl(Key::F),
        act: Act::SafeRect,
        own: true,
    },
    Binding {
        id: "COMMAND_DROP_TO_GROUND",
        group: "Object",
        label: "Drop to Ground",
        icon: Icon::None,
        keys: c(Key::End),
        act: Act::DropToGround,
        own: true,
    },
    Binding {
        id: "COMMAND_DELETE_SELECTION",
        group: "Edit",
        label: "Delete",
        icon: Icon::None,
        keys: c(Key::Delete),
        act: Act::Delete,
        own: false,
    },
    Binding {
        id: "COMMAND_SELECT_ALL",
        group: "Selection",
        label: "Select All",
        icon: Icon::None,
        keys: ctrl(Key::A),
        act: Act::SelectAll,
        own: false,
    },
    Binding {
        id: "COMMAND_DESELECT_ALL",
        group: "Selection",
        label: "Deselect All",
        icon: Icon::None,
        keys: c(Key::Escape),
        act: Act::DeselectAll,
        own: false,
    },
    Binding {
        id: "COMMAND_INVERT_SELECTION_ALL",
        group: "Selection",
        label: "Invert Selection",
        icon: Icon::None,
        keys: None,
        act: Act::InvertSelection,
        own: false,
    },
    Binding {
        id: "COMMAND_HIDE_SEL",
        group: "Visibility",
        label: "Hide Selected",
        icon: Icon::None,
        keys: c(Key::Minus),
        act: Act::HideSelected,
        own: false,
    },
    Binding {
        id: "COMMAND_HIDE_UNSEL",
        group: "Visibility",
        label: "Hide Unselected",
        icon: Icon::None,
        keys: c(Key::Equals),
        act: Act::HideUnselected,
        own: false,
    },
];

/// What each command of the table is bound to now: the author's binding, or the default.
#[must_use]
pub fn bound(editor: &Editor) -> Vec<(Act, Option<Chord>)> {
    let overrides = gz_app::EditorSettings::of(editor).shortcuts();
    BINDINGS
        .iter()
        .map(|b| {
            let keys = match overrides.iter().find(|(id, _)| id.eq_ignore_ascii_case(b.id)) {
                Some((_, text)) => Chord::parse(text),
                None => b.keys,
            };
            (b.act, keys)
        })
        .collect()
}

/// The keys of a command now, as a menu shows them ("Ctrl+G"); empty when it has none.
#[must_use]
pub fn keys_of(editor: &Editor, act: Act) -> String {
    bound(editor)
        .into_iter()
        .find(|(a, _)| *a == act)
        .and_then(|(_, k)| k)
        .map(|k| k.label())
        .unwrap_or_default()
}

/// The command bound to a chord, if any.
#[must_use]
pub fn act_of(editor: &Editor, chord: &Chord) -> Option<Act> {
    bound(editor).into_iter().find(|(_, k)| k.as_ref() == Some(chord)).map(|(a, _)| a)
}

/// The viewport's commands, for the Keyboard page and the palette (the scene's keys are also
/// answered by [`crate::ViewportPane::shortcut`] when the Level tab routes them).
pub fn install(b: &mut ShellBuilder) {
    for bind in BINDINGS.iter().filter(|b| b.own) {
        let act = bind.act;
        let mut cmd =
            UiCommand::new(bind.id, bind.group, bind.label, bind.icon, move |cx: &mut DocCx<'_>| {
                run(cx.editor, act)
            });
        cmd.default_keys = bind.keys;
        b.command(cmd.enabled(|_, tab| tab == "level"));
    }
}

/// Runs a command of the table.
pub fn run(editor: &Arc<Editor>, act: Act) {
    let vp = Viewport::of(editor);
    match act {
        Act::Gizmo(g) => vp.with(|s| {
            s.tool.action = Action::Select;
            s.tool.gizmo = g;
        }),
        Act::CycleGizmo => vp.with(|s| {
            s.tool.action = Action::Select;
            s.tool.gizmo = match s.tool.gizmo {
                GizmoType::Move => GizmoType::Rotate,
                GizmoType::Rotate => GizmoType::Scale,
                _ => GizmoType::Move,
            };
        }),
        Act::Place => vp.with(|s| s.tool.action = Action::Place),
        Act::StopPlacing => vp.with(|s| s.tool.action = Action::Select),
        Act::FrameAll | Act::FrameSelection => {
            let ed = editor.clone();
            editor.exec.spawn(async move {
                if !frame(&ed, act == Act::FrameSelection).await {
                    ed.log.error("Can't calculate bounding box. Nothing selected or some object unsupported this function.");
                }
            });
        }
        Act::Wireframe => vp.with(|s| {
            s.view.fill = if s.view.fill == Fill::Wireframe { Fill::Solid } else { Fill::Wireframe }
        }),
        Act::Grid => vp.with(|s| s.view.grid = !s.view.grid),
        Act::SafeRect => vp.with(|s| s.view.safe_rect = !s.view.safe_rect),
        Act::GridSmaller | Act::GridLarger => {
            // `CommandGridSlotSize`: whole metres, tenths below one
            let settings = gz_app::EditorSettings::of(editor);
            let val = settings.pref("grid_cell_size").unwrap_or(1.0) as f32;
            let mut step = 1.0f32;
            let next = if act == Act::GridLarger {
                if val < 1.0 {
                    step /= 10.0;
                }
                val + step
            } else {
                if (val - 1.0).abs() < 1e-5 || val < 1.0 {
                    step /= 10.0;
                }
                val - step
            };
            settings.set_pref("grid_cell_size", f64::from(next.clamp(0.1, 10.0)));
            settings.save_soon(editor);
            vp.sync(editor);
        }
        Act::DropToGround => editor.post("drop_objects", Args::new()),
        Act::Delete => scene_cmd(editor, SceneCmd::DeleteSelected),
        Act::DeselectAll => scene_cmd(editor, SceneCmd::SelectClass { class: None, select: false }),
        Act::SelectAll => {
            if let Ok(svc) = SceneService::of(editor) {
                let target = svc.store.snapshot().tools.target;
                svc.store
                    .post(SceneCmd::SelectClass { class: Some(target), select: true }, editor.ui_origin());
            }
        }
        Act::InvertSelection | Act::HideSelected | Act::HideUnselected => {
            let Ok(svc) = SceneService::of(editor) else { return };
            let snap = svc.store.snapshot();
            let target = snap.tools.target;
            let pick = |want_selected: bool| -> Vec<gz_scene::ObjId> {
                snap.of_class(target)
                    .filter(|o| o.flags.visible && snap.is_selected(o.id) == want_selected)
                    .map(|o| o.id)
                    .collect()
            };
            match act {
                Act::InvertSelection => {
                    let ids: Vec<_> =
                        snap.of_class(target).filter(|o| o.flags.visible).map(|o| o.id).collect();
                    svc.store.post(SceneCmd::Select { ids, mode: SelectMode::Toggle }, editor.ui_origin());
                }
                Act::HideSelected => {
                    svc.store.post(SceneCmd::Show { ids: pick(true), visible: false }, editor.ui_origin())
                }
                _ => svc.store.post(SceneCmd::Show { ids: pick(false), visible: false }, editor.ui_origin()),
            }
        }
    }
    editor.exec.wake().ping();
}

fn scene_cmd(editor: &Arc<Editor>, cmd: SceneCmd) {
    if let Ok(svc) = SceneService::of(editor) {
        svc.store.post(cmd, editor.ui_origin());
    }
}

/// Frame All / Frame Selection (`ZoomObject`): the current class's objects, or its selected
/// ones, in view. `false` when there is no box to frame.
pub async fn frame(editor: &Arc<Editor>, selected: bool) -> bool {
    let Ok(svc) = SceneService::of(editor) else { return false };
    let snap = svc.store.snapshot();
    let lib = svc.library.clone();
    let class: ObjClass = snap.tools.target;
    let found = editor.exec.cpu(move || crate::draw::class_box(&snap, &lib, class, selected)).await;
    let Some(b) = found else { return false };
    let vp = Viewport::of(editor);
    vp.with(|s| {
        let size = s.size;
        s.nav.zoom_extents(&b, size);
    });
    editor.exec.wake().ping();
    true
}
