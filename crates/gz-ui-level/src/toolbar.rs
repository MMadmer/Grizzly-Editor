//! The Level toolbar, lowest priority first: Save | Undo, Redo; Build Level; Play v, Play Clean,
//! Stop; Reload Configs, Game Folder.
//!
//! Each button is a command of the registry with its arguments; a button whose command is not
//! registered is not added. The level builds and the runs are `gz-levelbuild`'s.

use std::sync::Arc;

use gz_editor::{Args, Editor, Value};
use gz_scene::SceneService;
use gz_ui::{Icon, Kind, MenuItem, Toolbar, menu_item};

/// A button of the level builds and runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelAction {
    /// The toolbar item's id.
    pub id: &'static str,
    pub icon: Icon,
    /// Shown beside the icon; empty for an icon-only button.
    pub label: &'static str,
    /// What it does, for the tip and the menus.
    pub name: &'static str,
    pub tip: &'static str,
    /// The registry command it runs.
    pub command: &'static str,
    /// The command's arguments.
    pub args: &'static [(&'static str, &'static str)],
    /// The toolbar group, lowest priority first.
    pub group: i32,
}

#[allow(clippy::too_many_arguments)]
const fn act(
    id: &'static str,
    icon: Icon,
    label: &'static str,
    name: &'static str,
    tip: &'static str,
    command: &'static str,
    args: &'static [(&'static str, &'static str)],
    group: i32,
) -> LevelAction {
    LevelAction { id, icon, label, name, tip, command, args, group }
}

/// The level builds and the runs, as `gz-levelbuild` registers them.
pub const LEVEL_ACTIONS: [LevelAction; 5] = [
    act(
        "build_level",
        Icon::BuildAll,
        "Build Level",
        "Build Level",
        "Build what the scene's changes left stale, step by step",
        "level.build",
        &[],
        1,
    ),
    act(
        "play",
        Icon::Play,
        "Play",
        "Play",
        "Build what is stale and start the game on the level",
        "level.play",
        &[("action", "start")],
        2,
    ),
    act(
        "play_clean",
        Icon::PlayClean,
        "",
        "Play Clean",
        "Start the game as it is, without the scene and without building anything",
        "level.play",
        &[("action", "start"), ("clean", "true")],
        2,
    ),
    act(
        "stop",
        Icon::Stop,
        "",
        "Stop",
        "Stop the build, the compiler or the game that is running",
        "level.play",
        &[("action", "stop")],
        2,
    ),
    act(
        "reload_configs",
        Icon::ReloadConfigs,
        "",
        "Reload Configs",
        "Read the linked game's configs again (system.ltx, game.ltx)",
        "level.play",
        &[("action", "reload_configs")],
        3,
    ),
];

/// The job groups the level builds and the runs use: while one runs, the others wait.
const RUN_GROUPS: [&str; 2] = ["level_build", "game"];

/// A compiler or the game is running.
#[must_use]
pub fn running(editor: &Editor) -> bool {
    RUN_GROUPS.iter().any(|g| editor.jobs.running_in(g).is_some())
}

const BUSY: &str = "A compiler or the game is running.";

/// The action of a toolbar id.
#[must_use]
pub fn action(id: &str) -> Option<&'static LevelAction> {
    LEVEL_ACTIONS.iter().find(|a| a.id == id)
}

/// Runs a level action's command when it is registered, its own arguments under `args`.
pub fn run(editor: &Arc<Editor>, a: &LevelAction, mut args: Args) -> bool {
    if !editor.registry().has(a.command) {
        return false;
    }
    for (k, v) in a.args {
        args.entry((*k).to_owned()).or_insert_with(|| Value::String((*v).to_owned()));
    }
    editor.post(a.command, args);
    true
}

/// What the Play button's menu sets.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolbarState {
    pub verify_restrictors: bool,
}

/// Draws the Level toolbar. `save` and `save_as` are what the Save button does.
pub fn draw(tb: &mut Toolbar<'_, '_>, editor: &Arc<Editor>, st: &mut ToolbarState, save: &mut bool) {
    let busy = running(editor);
    let has = |c: &str| editor.registry().has(c);
    let history =
        editor.project_opt().and_then(|s| s.services.get::<SceneService>()).map(|svc| svc.store.history());
    let (can_undo, can_redo) =
        history.as_ref().map_or((false, false), |h| (!h.undo.is_empty(), !h.redo.is_empty()));

    tb.group(0);
    if has("scene.save")
        && tb.action("save", Icon::Save, "", "Save the scene (Ctrl+S)", true, false, Kind::Ghost)
    {
        *save = true;
    }
    tb.separator();
    if tb.action("undo", Icon::Undo, "", "Undo the last change (Ctrl+Z)", can_undo, false, Kind::Ghost) {
        editor.post("doc.undo", Args::from_iter([("doc".to_owned(), gz_editor::Value::from("level"))]));
    }
    if tb.action(
        "redo",
        Icon::Redo,
        "",
        "Redo the change that was undone (Ctrl+Y)",
        can_redo,
        false,
        Kind::Ghost,
    ) {
        editor.post("doc.redo", Args::from_iter([("doc".to_owned(), gz_editor::Value::from("level"))]));
    }

    let mut group = 0;
    for a in &LEVEL_ACTIONS {
        if !has(a.command) {
            continue;
        }
        if a.group != group {
            group = a.group;
            tb.group(group);
        }
        let stop = a.id == "stop";
        let on = if stop { busy } else { !busy };
        let tip = if stop {
            if busy { a.tip } else { "Nothing is running" }
        } else if busy {
            BUSY
        } else {
            a.tip
        };
        let kind = Kind::Ghost;
        if tb.action(a.id, a.icon, a.label, tip, on, false, kind) {
            let mut args = Args::new();
            if a.id == "play" {
                args.insert("verify_restrictors".into(), Value::Bool(st.verify_restrictors));
            }
            run(editor, a, args);
        }
        if a.id == "play" {
            let verify = &mut st.verify_restrictors;
            tb.menu(
                "play_options",
                Icon::None,
                "",
                "Options for playing",
                Some("toolbar_split:play"),
                true,
                |ui| {
                    if menu_item(
                        ui,
                        "Verify Space Restrictors",
                        MenuItem { selected: *verify, ..MenuItem::default() },
                    ) {
                        *verify = !*verify;
                    }
                },
            );
        }
    }
    if group != 3 {
        tb.group(3);
    }
    let root = gz_game::GameLink::of(editor).ok().map(|l| l.root()).filter(|r| !r.is_empty());
    let why = if root.is_some() {
        "Open the gamedata folder of the linked game in Explorer"
    } else {
        "No game is linked"
    };
    if tb.action("game_folder", Icon::GameFolder, "", why, root.is_some(), false, Kind::Ghost)
        && let Some(root) = root
    {
        let folder = std::path::Path::new(&root).join("gamedata");
        let target = if folder.is_dir() { folder } else { std::path::PathBuf::from(root) };
        let log = editor.log.clone();
        editor.exec.spawn_io(move || {
            if let Err(e) = std::process::Command::new("explorer").arg(&target).spawn() {
                log.warn(format!("Explorer did not open {}: {e}", target.display()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_actions_are_in_toolbar_order() {
        let groups: Vec<i32> = LEVEL_ACTIONS.iter().map(|a| a.group).collect();
        assert!(groups.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(
            action("stop").map(|a| (a.command, a.args)),
            Some(("level.play", &[("action", "stop")][..]))
        );
    }
}
