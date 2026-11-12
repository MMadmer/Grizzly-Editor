//! The Level toolbar (`UITopBarForm::DrawLevel`), lowest priority first: Save | Undo, Redo;
//! Build Collision, Build AI Map, Build Game Graph, Build All; Play v, Play in Game, Play
//! Clean, Stop; Reload Configs, Game Folder.
//!
//! Each button is a command of the registry; a button whose command is not registered is not
//! added. The level builds and the runs are `gz-levelbuild`'s ([`LEVEL_ACTIONS`] names the
//! commands it registers).

use std::sync::Arc;

use gz_editor::{Args, Editor, Value};
use gz_scene::SceneService;
use gz_ui::{Icon, Kind, MenuItem, Toolbar, menu_item};

/// A button of the level builds and runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelAction {
    /// The toolbar item's id (the C++ one).
    pub id: &'static str,
    pub icon: Icon,
    /// Shown beside the icon; empty for an icon-only button.
    pub label: &'static str,
    /// What it does, for the tip and the menus.
    pub name: &'static str,
    pub tip: &'static str,
    /// The registry command it runs.
    pub command: &'static str,
    /// The toolbar group, lowest priority first.
    pub group: i32,
}

const fn act(
    id: &'static str,
    icon: Icon,
    label: &'static str,
    name: &'static str,
    tip: &'static str,
    command: &'static str,
    group: i32,
) -> LevelAction {
    LevelAction { id, icon, label, name, tip, command, group }
}

/// The level builds and the runs, as `gz-levelbuild` registers them.
pub const LEVEL_ACTIONS: [LevelAction; 9] = [
    act(
        "build_cform",
        Icon::BuildCollision,
        "",
        "Build Collision",
        "Build the collision geometry (CForm) of this scene",
        "build_cform",
        1,
    ),
    act(
        "build_aimap",
        Icon::BuildAIMap,
        "",
        "Build AI Map",
        "Build the AI map of this scene",
        "build_aimap",
        1,
    ),
    act(
        "build_ggraph",
        Icon::BuildGameGraph,
        "",
        "Build Game Graph",
        "Build the game graph of this scene",
        "build_game_graph",
        1,
    ),
    act(
        "build_all",
        Icon::BuildAll,
        "",
        "Build All",
        "Make the level build and compile it: geometry and light",
        "build_all",
        1,
    ),
    act(
        "play",
        Icon::Play,
        "Play",
        "Play",
        "Play the scene: build what it needs and launch the game on it",
        "play",
        2,
    ),
    act(
        "play_game",
        Icon::PlayGame,
        "",
        "Play in Game",
        "Build the scene for play and launch the game on it",
        "play_game",
        2,
    ),
    act(
        "play_clean",
        Icon::PlayClean,
        "",
        "Play Clean",
        "Launch the game as it is, without building anything",
        "play_clean",
        2,
    ),
    act("stop", Icon::Stop, "", "Stop", "Stop the compiler or the game that is running", "play_stop", 2),
    act(
        "reload_configs",
        Icon::ReloadConfigs,
        "",
        "Reload Configs",
        "Re-read the game configs from disk (system.ltx, game.ltx)",
        "reload_configs",
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

/// Runs a level action's command when it is registered.
pub fn run(editor: &Arc<Editor>, a: &LevelAction, args: Args) -> bool {
    if !editor.registry().has(a.command) {
        return false;
    }
    editor.post(a.command, args);
    true
}

/// The toolbar's state the C++ kept on the form.
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
    if has("save_scene")
        && tb.action("save", Icon::Save, "", "Save the scene (Ctrl+S)", true, false, Kind::Ghost)
    {
        *save = true;
    }
    tb.separator();
    if has("undo")
        && tb.action("undo", Icon::Undo, "", "Undo the last change (Ctrl+Z)", can_undo, false, Kind::Ghost)
    {
        editor.post("undo", Args::new());
    }
    if has("redo")
        && tb.action(
            "redo",
            Icon::Redo,
            "",
            "Redo the change that was undone (Ctrl+Y)",
            can_redo,
            false,
            Kind::Ghost,
        )
    {
        editor.post("redo", Args::new());
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
        assert_eq!(action("stop").map(|a| a.command), Some("play_stop"));
    }
}
