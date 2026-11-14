//! The graph tab's actions (the group "Graph"): what its keys, its menus and the palette run.
//! Each runs one of the graph's commands on the tab that is up - `graph.edit` or `graph.arrange`
//! on the selection, `graph.view` for what the view does, `graph.find` for the search - so each
//! is in the palette, on the Keyboard page and in `help`, and every key can be bound again. The
//! keys of those that act on the canvas are live while the canvas has them
//! (`Scope::Part(.., "canvas")`).
//!
//! The pattern for a new one: a name `graph.<verb>`, a title, default keys, the scope and the
//! command with its arguments.

use egui::Key;
use gz_app::{Action, ActionCx, Chord, Enabled, Scope, ShellBuilder};
use gz_editor::{Args, Value, json};
use gz_nq::NqService;
use gz_nq::canvas::{Flow, WireStyle};
use gz_nq::doc::GraphDoc;
use gz_ui::Icon;

/// The kinds of tab a graph action lives on.
pub const GRAPHS: &[&str] = &["quest", "beh"];
const CANVAS: Scope = Scope::Part(GRAPHS, "canvas");
const TAB: Scope = Scope::Tab(GRAPHS);

fn on_a_graph(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(GRAPHS.contains(&cx.kind), "Open a quest or a behaviour graph first.")
}

/// What the open document of the graph that is up says.
fn with_doc<R>(cx: &ActionCx<'_>, f: impl FnOnce(&GraphDoc) -> R) -> Option<R> {
    if !GRAPHS.contains(&cx.kind) {
        return None;
    }
    let snap = NqService::of(cx.editor).ok()?.get_doc(cx.path)?.store.snapshot();
    Some(f(&snap))
}

fn on_selection(cx: &ActionCx<'_>) -> Enabled {
    if !GRAPHS.contains(&cx.kind) {
        return on_a_graph(cx);
    }
    Enabled::when(with_doc(cx, |s| !s.view.selection.is_empty()).unwrap_or(false), "Select nodes first.")
}

fn on_two(cx: &ActionCx<'_>) -> Enabled {
    if !GRAPHS.contains(&cx.kind) {
        return on_a_graph(cx);
    }
    Enabled::when(with_doc(cx, |s| s.view.selection.len() >= 2).unwrap_or(false), "Select two nodes or more.")
}

fn on_three(cx: &ActionCx<'_>) -> Enabled {
    if !GRAPHS.contains(&cx.kind) {
        return on_a_graph(cx);
    }
    Enabled::when(
        with_doc(cx, |s| s.view.selection.len() >= 3).unwrap_or(false),
        "Select three nodes or more.",
    )
}

fn on_one(cx: &ActionCx<'_>) -> Enabled {
    if !GRAPHS.contains(&cx.kind) {
        return on_a_graph(cx);
    }
    Enabled::when(with_doc(cx, |s| s.view.selection.len() == 1).unwrap_or(false), "Select one node.")
}

fn flows_right(cx: &ActionCx<'_>) -> bool {
    with_doc(cx, |s| s.graph.flow() == Flow::Right).unwrap_or(false)
}

fn flows_down(cx: &ActionCx<'_>) -> bool {
    with_doc(cx, |s| s.graph.flow() == Flow::Down).unwrap_or(false)
}

fn own_style(cx: &ActionCx<'_>) -> Option<WireStyle> {
    with_doc(cx, |s| s.graph.canvas.wires.style).flatten()
}

/// An action of the group Graph: `$cmd` with `doc` (the tab that is up) and the arguments given.
macro_rules! act {
    ($id:literal, $title:literal, $cmd:literal, $scope:expr, $icon:expr, $keys:expr, $on:expr, { $($k:literal : $v:tt),* $(,)? }) => {
        Action::new($id, $title, "Graph", $cmd)
            .icon($icon)
            .keys($keys)
            .scope($scope)
            .args(|cx| {
                let mut a = Args::new();
                a.insert("doc".into(), Value::String(cx.tab.to_owned()));
                $( a.insert($k.into(), json!($v)); )*
                a
            })
            .enabled($on)
    };
}

/// The bookmark actions: Ctrl+1..9 jumps to bookmark 1..9 (by its name, else its place),
/// Ctrl+Shift+1..9 sets it on the selected node.
macro_rules! bookmark_pair {
    ($n:literal, $key:expr, $jump:literal, $set:literal, $jt:literal, $st:literal) => {
        [
            act!($jump, $jt, "graph.view", TAB, Icon::BookmarkNext, const { &[Chord::ctrl($key)] }, on_a_graph, { "bookmark": $n }),
            act!($set, $st, "graph.edit", CANVAS, Icon::Bookmark, const { &[Chord::ctrl_shift($key)] }, on_one, {
                "ops": [{ "op": "bookmark", "name": $n, "here": true }]
            }),
        ]
    };
}

/// Every action of the graph tab.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn actions() -> Vec<Action> {
    let mut v = vec![
        // the view
        act!("graph.frame_all", "Frame All", "graph.view", CANVAS, Icon::ZoomFit, const { &[Chord::new(Key::Home)] }, on_a_graph, { "frame": "all" }),
        act!("graph.frame_selection", "Frame Selection", "graph.view", CANVAS, Icon::FrameSelection, const { &[Chord::new(Key::F)] }, on_a_graph, { "frame": "selection" }),
        act!("graph.back", "Back", "graph.view", TAB, Icon::Back, const { &[Chord::alt(Key::ArrowLeft)] }, on_a_graph, { "history": "back" }),
        act!("graph.forward", "Forward", "graph.view", TAB, Icon::Forward, const { &[Chord::alt(Key::ArrowRight)] }, on_a_graph, { "history": "forward" }),
        act!("graph.add_node", "Add Node...", "graph.view", CANVAS, Icon::Plus, const { &[Chord::new(Key::Tab), Chord::new(Key::Space)] }, on_a_graph, { "menu": "picker" }),
        act!("graph.replace_node", "Replace Node...", "graph.view", CANVAS, Icon::Refresh, &[], on_one, { "menu": "replace" }),
        act!("graph.rename", "Rename Node", "graph.view", CANVAS, Icon::Rename, const { &[Chord::new(Key::F2)] }, on_one, { "focus": "rename" }),
        act!("graph.remove_wire", "Remove the Wire Under the Pointer", "graph.view", CANVAS, Icon::Unlink, const { &[Chord::new(Key::D)] }, on_a_graph, { "wire": "remove_hovered" }),
        act!("graph.select_all", "Select All Nodes", "graph.view", CANVAS, Icon::Check, const { &[Chord::ctrl(Key::A)] }, on_a_graph, { "select": "all" }),
        act!("graph.select_connected", "Select Connected", "graph.view", CANVAS, Icon::Link, const { &[Chord::ctrl(Key::L)] }, on_selection, { "grow": "connected" }),
        act!("graph.select_upstream", "Select Upstream", "graph.view", CANVAS, Icon::Up, &[], on_selection, { "grow": "upstream" }),
        act!("graph.select_downstream", "Select Downstream", "graph.view", CANVAS, Icon::ChevronDown, &[], on_selection, { "grow": "downstream" }),
        act!("graph.previous_bookmark", "Previous Bookmark", "graph.view", TAB, Icon::BookmarkPrev, &[], on_a_graph, { "bookmark": "previous" }),
        act!("graph.next_bookmark", "Next Bookmark", "graph.view", TAB, Icon::BookmarkNext, &[], on_a_graph, { "bookmark": "next" }),
        act!("graph.find", "Find", "graph.view", TAB, Icon::Search, const { &[Chord::ctrl(Key::F)] }, on_a_graph, { "drawer": "find" }),
        act!("graph.find_in_project", "Find in Project", "graph.view", TAB, Icon::FindInProject, const { &[Chord::ctrl_shift(Key::F)] }, on_a_graph, { "dialog": "find_in_project" }),
        act!("graph.problems", "Show Problems", "graph.view", TAB, Icon::Problems, &[], on_a_graph, { "drawer": "problems" }),
        act!("graph.find_next", "Find Next", "graph.find", TAB, Icon::Search, const { &[Chord::new(Key::F3)] }, on_a_graph, { "go": "next" }),
        act!("graph.find_previous", "Find Previous", "graph.find", TAB, Icon::Search, const { &[Chord::shift(Key::F3)] }, on_a_graph, { "go": "previous" }),
        // edits of the selection
        act!("graph.connect_nearest", "Connect to the Nearest", "graph.edit", CANVAS, Icon::Link, const { &[Chord::new(Key::Q)] }, on_one, { "ops": [{ "op": "connect_nearest" }] }),
        act!("graph.bypass", "Bypass Node", "graph.edit", CANVAS, Icon::Unlink, const { &[Chord::alt(Key::D)] }, on_selection, { "ops": [{ "op": "bypass" }] }),
        act!("graph.duplicate_with_wires", "Duplicate with Wires", "graph.edit", CANVAS, Icon::Duplicate, const { &[Chord::ctrl_shift(Key::D)] }, on_selection, { "ops": [{ "op": "duplicate", "wires": true }] }),
        act!("graph.disable", "Disable / Enable", "graph.edit", CANVAS, Icon::EyeOff, const { &[Chord::ctrl_shift(Key::E)] }, on_selection, { "ops": [{ "op": "disable" }] }),
        act!("graph.collapse", "Collapse / Expand", "graph.edit", CANVAS, Icon::CollapseAll, const { &[Chord::ctrl_shift(Key::H)] }, on_selection, { "ops": [{ "op": "collapse" }] }),
        act!("graph.bookmark", "Bookmark the Node", "graph.edit", CANVAS, Icon::Bookmark, const { &[Chord::ctrl(Key::B)] }, on_one, { "ops": [{ "op": "bookmark", "here": true }] }),
        act!("graph.add_frame", "Add Comment Frame", "graph.edit", CANVAS, Icon::Group, const { &[Chord::new(Key::C)] }, on_selection, { "ops": [{ "op": "frame", "action": "add", "title": "Comment" }] }),
        // arrangements
        act!("graph.auto_layout", "Auto Layout", "graph.arrange", TAB, Icon::AutoLayout, &[], on_a_graph, { "action": "layout", "scope": "all" }),
        act!("graph.auto_layout_selection", "Auto Layout Selection", "graph.arrange", CANVAS, Icon::AutoLayout, &[], on_two, { "action": "layout", "scope": "selection" }),
        act!("graph.auto_layout_branch", "Auto Layout Branch", "graph.arrange", CANVAS, Icon::AutoLayout, &[], on_one, { "action": "layout", "scope": "branch" }),
        act!("graph.align_left", "Align Left", "graph.arrange", CANVAS, Icon::ChevronLeft, const { &[Chord::shift(Key::A)] }, on_two, { "action": "align", "how": "left" }),
        act!("graph.align_centre", "Align Centre", "graph.arrange", CANVAS, Icon::Minus, &[], on_two, { "action": "align", "how": "centre" }),
        act!("graph.align_right", "Align Right", "graph.arrange", CANVAS, Icon::ChevronRight, const { &[Chord::shift(Key::D)] }, on_two, { "action": "align", "how": "right" }),
        act!("graph.align_top", "Align Top", "graph.arrange", CANVAS, Icon::ChevronUp, const { &[Chord::shift(Key::W)] }, on_two, { "action": "align", "how": "top" }),
        act!("graph.align_middle", "Align Middle", "graph.arrange", CANVAS, Icon::Minus, &[], on_two, { "action": "align", "how": "middle" }),
        act!("graph.align_bottom", "Align Bottom", "graph.arrange", CANVAS, Icon::ChevronDown, const { &[Chord::shift(Key::S)] }, on_two, { "action": "align", "how": "bottom" }),
        act!("graph.distribute_horizontally", "Distribute Horizontally", "graph.arrange", CANVAS, Icon::More, &[], on_three, { "action": "distribute", "how": "horizontal" }),
        act!("graph.distribute_vertically", "Distribute Vertically", "graph.arrange", CANVAS, Icon::Overflow, &[], on_three, { "action": "distribute", "how": "vertical" }),
        act!("graph.straighten", "Straighten Connections", "graph.arrange", CANVAS, Icon::Flow, const { &[Chord::shift(Key::Q)] }, on_selection, { "action": "straighten" }),
        act!("graph.flow_right", "Flow Left to Right", "graph.arrange", TAB, Icon::ChevronRight, &[], on_a_graph, { "action": "layout", "flow": "right" })
            .checked(flows_right),
        act!("graph.flow_down", "Flow Top to Bottom", "graph.arrange", TAB, Icon::ChevronDown, &[], on_a_graph, { "action": "layout", "flow": "down" })
            .checked(flows_down),
        // the graph's own wires over the preferences
        act!("graph.wires_default", "Wires as in Preferences", "graph.edit", TAB, Icon::Settings, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "default" }] })
            .checked(|cx| with_doc(cx, |s| s.graph.canvas.wires.style.is_none()).unwrap_or(false)),
        act!("graph.wires_curved", "Curved Wires", "graph.edit", TAB, Icon::Flow, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "curved" }] })
            .checked(|cx| own_style(cx) == Some(WireStyle::Curved)),
        act!("graph.wires_straight", "Straight Wires", "graph.edit", TAB, Icon::Flow, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "straight" }] })
            .checked(|cx| own_style(cx) == Some(WireStyle::Straight)),
        act!("graph.wires_circuit", "Circuit Wires", "graph.edit", TAB, Icon::Flow, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "circuit" }] })
            .checked(|cx| own_style(cx) == Some(WireStyle::Circuit)),
        act!("graph.wires_angled", "Angled Wires", "graph.edit", TAB, Icon::Flow, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "angled" }] })
            .checked(|cx| own_style(cx) == Some(WireStyle::Angled)),
        act!("graph.wires_subway", "Subway Wires", "graph.edit", TAB, Icon::Flow, &[], on_a_graph, { "ops": [{ "op": "wires", "style": "subway" }] })
            .checked(|cx| own_style(cx) == Some(WireStyle::Subway)),
        Action::new("graph.reload", "Reload", "Graph", "doc.reload")
            .icon(Icon::Reload)
            .scope(TAB)
            .args(|cx| {
                let mut a = Args::new();
                a.insert("doc".into(), Value::String(cx.tab.to_owned()));
                a
            })
            .enabled(on_a_graph),
    ];
    for pair in [
        bookmark_pair!(
            "1",
            Key::Num1,
            "graph.bookmark_1",
            "graph.set_bookmark_1",
            "Go to Bookmark 1",
            "Set Bookmark 1"
        ),
        bookmark_pair!(
            "2",
            Key::Num2,
            "graph.bookmark_2",
            "graph.set_bookmark_2",
            "Go to Bookmark 2",
            "Set Bookmark 2"
        ),
        bookmark_pair!(
            "3",
            Key::Num3,
            "graph.bookmark_3",
            "graph.set_bookmark_3",
            "Go to Bookmark 3",
            "Set Bookmark 3"
        ),
        bookmark_pair!(
            "4",
            Key::Num4,
            "graph.bookmark_4",
            "graph.set_bookmark_4",
            "Go to Bookmark 4",
            "Set Bookmark 4"
        ),
        bookmark_pair!(
            "5",
            Key::Num5,
            "graph.bookmark_5",
            "graph.set_bookmark_5",
            "Go to Bookmark 5",
            "Set Bookmark 5"
        ),
        bookmark_pair!(
            "6",
            Key::Num6,
            "graph.bookmark_6",
            "graph.set_bookmark_6",
            "Go to Bookmark 6",
            "Set Bookmark 6"
        ),
        bookmark_pair!(
            "7",
            Key::Num7,
            "graph.bookmark_7",
            "graph.set_bookmark_7",
            "Go to Bookmark 7",
            "Set Bookmark 7"
        ),
        bookmark_pair!(
            "8",
            Key::Num8,
            "graph.bookmark_8",
            "graph.set_bookmark_8",
            "Go to Bookmark 8",
            "Set Bookmark 8"
        ),
        bookmark_pair!(
            "9",
            Key::Num9,
            "graph.bookmark_9",
            "graph.set_bookmark_9",
            "Go to Bookmark 9",
            "Set Bookmark 9"
        ),
    ] {
        v.extend(pair);
    }
    v
}

/// Adds the graph's actions to the shell.
pub fn install(b: &mut ShellBuilder) {
    for a in actions() {
        b.action(a);
    }
}

#[cfg(test)]
mod tests {
    use super::actions;

    /// Every action runs one of the graph's commands (or the shell's reload), lives in the group
    /// Graph, and no two take the same key on the same part of the tab.
    #[test]
    fn every_action_runs_a_graph_command() {
        let all = actions();
        for a in &all {
            assert!(
                ["graph.edit", "graph.arrange", "graph.view", "graph.find", "doc.reload"]
                    .contains(&a.command),
                "{} runs {}",
                a.id,
                a.command
            );
            assert_eq!(a.group, "Graph");
        }
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
                if a.scope.overlaps(b.scope) {
                    assert!(a.keys.iter().all(|k| !b.keys.contains(k)), "{} and {} share a key", a.id, b.id);
                }
            }
        }
    }
}
