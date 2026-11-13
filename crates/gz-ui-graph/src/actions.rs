//! The graph tab's actions (the group "Graph"): what its keys, its menus and the palette run. Each
//! is `graph.action` with the name of what it does, on the tab that is up; the keys of those that
//! act on the canvas are live while the canvas has them (`Scope::Part(.., "canvas")`).
//!
//! The pattern for a new one: a name `graph.<verb>`, a title, default keys, the scope, and the
//! arm of [`GraphTab::run_named`] that does it - through the tab's own queue, as a key did.

use std::sync::Arc;

use egui::Key;
use gz_app::{Action, ActionCx, Chord, Enabled, Scope, ShellBuilder};
use gz_editor::{CmdError, Editor, Origin, Registry, Reply, Spec, Value, json};
use gz_ui::Icon;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::tab::GraphTab;

/// The kinds of tab a graph action lives on.
pub const GRAPHS: &[&str] = &["quest", "beh"];
const CANVAS: Scope = Scope::Part(GRAPHS, "canvas");
const TAB: Scope = Scope::Tab(GRAPHS);

fn on_a_graph(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(GRAPHS.contains(&cx.kind), "Open a quest or a behaviour graph first.")
}

macro_rules! graph_action {
    ($id:literal, $title:literal, $what:literal, $scope:expr, $icon:expr, $keys:expr) => {
        Action::new($id, $title, "Graph", "graph.action")
            .icon($icon)
            .keys($keys)
            .scope($scope)
            .args(|cx| {
                let mut a = gz_editor::Args::new();
                a.insert("doc".into(), Value::String(cx.tab.to_owned()));
                a.insert("what".into(), Value::String($what.to_owned()));
                a
            })
            .enabled(on_a_graph)
    };
}

/// Every action of the graph tab.
#[must_use]
pub fn actions() -> Vec<Action> {
    vec![
        graph_action!(
            "graph.frame_all",
            "Frame All",
            "frame_all",
            CANVAS,
            Icon::ZoomFit,
            const { &[Chord::new(Key::Home)] }
        ),
        graph_action!(
            "graph.frame_selection",
            "Frame Selection",
            "frame_selection",
            CANVAS,
            Icon::FrameSelection,
            const { &[Chord::new(Key::F)] }
        ),
        graph_action!(
            "graph.rename",
            "Rename Node",
            "rename",
            CANVAS,
            Icon::Rename,
            const { &[Chord::new(Key::F2)] }
        ),
        graph_action!(
            "graph.connect_nearest",
            "Connect to the Nearest",
            "connect_nearest",
            CANVAS,
            Icon::Link,
            const { &[Chord::new(Key::Q)] }
        ),
        graph_action!(
            "graph.bypass",
            "Bypass Node",
            "bypass",
            CANVAS,
            Icon::Unlink,
            const { &[Chord::alt(Key::D)] }
        ),
        graph_action!(
            "graph.remove_wire",
            "Remove the Wire Under the Pointer",
            "remove_wire",
            CANVAS,
            Icon::Delete,
            const { &[Chord::new(Key::D)] }
        ),
        graph_action!(
            "graph.select_all",
            "Select All Nodes",
            "select_all",
            CANVAS,
            Icon::Check,
            const { &[Chord::ctrl(Key::A)] }
        ),
        graph_action!(
            "graph.bookmark",
            "Bookmark the Selection",
            "bookmark",
            CANVAS,
            Icon::Bookmark,
            const { &[Chord::ctrl(Key::B)] }
        ),
        graph_action!(
            "graph.previous_bookmark",
            "Previous Bookmark",
            "previous_bookmark",
            TAB,
            Icon::BookmarkPrev,
            &[]
        ),
        graph_action!("graph.next_bookmark", "Next Bookmark", "next_bookmark", TAB, Icon::BookmarkNext, &[]),
        graph_action!("graph.back", "Back", "back", TAB, Icon::Back, const { &[Chord::alt(Key::ArrowLeft)] }),
        graph_action!(
            "graph.forward",
            "Forward",
            "forward",
            TAB,
            Icon::Forward,
            const { &[Chord::alt(Key::ArrowRight)] }
        ),
        graph_action!(
            "graph.find_next",
            "Find Next",
            "find_next",
            TAB,
            Icon::Search,
            const { &[Chord::new(Key::F3)] }
        ),
        graph_action!(
            "graph.find_previous",
            "Find Previous",
            "find_previous",
            TAB,
            Icon::Search,
            const { &[Chord::shift(Key::F3)] }
        ),
        graph_action!(
            "graph.find_in_project",
            "Find in Project",
            "find_in_project",
            TAB,
            Icon::FindInProject,
            const { &[Chord::ctrl_shift(Key::F)] }
        ),
        graph_action!("graph.auto_layout", "Auto Layout", "auto_layout", TAB, Icon::AutoLayout, &[]),
        graph_action!("graph.validate", "Validate", "validate", TAB, Icon::Validate, &[]),
        graph_action!("graph.reload", "Reload", "reload", TAB, Icon::Reload, &[]),
    ]
}

/// Adds the graph's actions to the shell.
pub fn install(b: &mut ShellBuilder) {
    for a in actions() {
        b.action(a);
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActionParams {
    /// What to do, as the graph's actions name it: frame_all, frame_selection, rename,
    /// connect_nearest, bypass, remove_wire, select_all, bookmark, previous_bookmark,
    /// next_bookmark, back, forward, find_next, find_previous, find_in_project, auto_layout,
    /// validate, reload.
    what: String,
    /// The graph's tab (`quest:<path>`, `beh:<path>`). Left out: the tab that is up.
    #[serde(default)]
    doc: Option<String>,
}

async fn graph_action(ed: Arc<Editor>, p: ActionParams, _origin: Origin) -> Reply {
    gz_app::on_ui(&ed, move |shell, ctx| {
        let id = p.doc.filter(|d| !d.is_empty()).unwrap_or_else(|| shell.active().to_owned());
        let tab = shell
            .document_mut(&id)
            .and_then(|d| d.as_any_mut())
            .and_then(|a| a.downcast_mut::<GraphTab>())
            .ok_or_else(|| CmdError::not_found(format!("'{id}' is no open graph (ui.tab lists the tabs)")))?;
        tab.run_named(ctx, &p.what).map_err(CmdError::invalid)?;
        ctx.request_repaint();
        Ok(json!({ "doc": id, "done": p.what }))
    })
    .await
}

/// Registers `graph.action`.
pub fn register(reg: &mut Registry) {
    reg.area("graph", "An open quest or behaviour graph: its actions, its view");
    reg.cmd(
        Spec::write("graph.action", "Runs one of the graph tab's actions on what it shows, as its key does.")
            .example("what=frame_all"),
        graph_action,
    );
}

#[cfg(test)]
mod tests {
    use super::actions;

    #[test]
    fn every_action_names_a_thing_the_tab_does() {
        let names = crate::tab::GraphTab::NAMED;
        for a in actions() {
            let what = a.id.trim_start_matches("graph.");
            assert!(names.contains(&what), "{} runs nothing", a.id);
            assert_eq!(a.group, "Graph");
        }
    }
}
