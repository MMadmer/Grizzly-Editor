//! The project area's groups of Preferences: **Game** (the editor's default game - its folder,
//! version, how it was found, Change..., Find Again - and the open project's own, with Use
//! Default Game), **Library** (the folder scenes take what the project lacks from) and, on
//! General, **Project Files** (Open .grizzly Files with This Editor, Register Now).
//!
//! Every change is a command (`game.link`, `project.set_library`, `project.associate`). Nothing
//! here reads a file in a frame: what the rows show is the last `game.info` / `project.info` /
//! `project.associate` answer, asked again off the UI thread when it is two seconds old or a
//! change went through.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gz_app::{Enabled, PrefGroup, PrefKind, PrefRow, ShellBuilder};
use gz_editor::Editor;
use gz_ui::Icon;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::folder::pick_folder;
use crate::reply::{args, call_then, error_text, ok, s};

/// The answers the rows show.
#[derive(Default)]
struct Shown {
    game: Value,
    project: Value,
    association: Value,
    /// When they were asked for, and whether an ask is out.
    asked: Option<Instant>,
    asking: bool,
    /// What the last change said when it failed.
    error: String,
}

/// The rows' service.
#[derive(Default)]
pub struct GamePrefs {
    shown: Mutex<Shown>,
}

impl GamePrefs {
    fn of(editor: &Editor) -> Arc<GamePrefs> {
        editor.services.get_or_init(GamePrefs::default)
    }

    /// Asks the three answers again when they are old (or `now`): off the UI thread.
    fn refresh(self: &Arc<Self>, editor: &Arc<Editor>, now: bool) {
        {
            let mut s = self.shown.lock();
            let fresh = s.asked.is_some_and(|t| t.elapsed() < Duration::from_secs(2));
            if s.asking || (fresh && !now) {
                return;
            }
            s.asking = true;
        }
        let (ed, me) = (editor.clone(), self.clone());
        let _task = editor.exec.spawn(async move {
            let origin = gz_editor::Origin::Mcp;
            let game = ed.call("game.info", gz_editor::Args::new(), origin).await;
            let project = ed.call("project.info", gz_editor::Args::new(), origin).await;
            let association = ed.call("project.associate", gz_editor::Args::new(), origin).await;
            let mut s = me.shown.lock();
            s.game = game;
            s.project = project;
            s.association = association;
            s.asked = Some(Instant::now());
            s.asking = false;
            ed.exec.wake().ping();
        });
    }

    /// A change through `command`; the rows are asked again after it.
    fn change(self: &Arc<Self>, editor: &Arc<Editor>, command: &'static str, a: gz_editor::Args) {
        let (ed, me) = (editor.clone(), self.clone());
        call_then(editor, command, a, move |reply, _| {
            me.shown.lock().error = if ok(reply) { String::new() } else { error_text(reply) };
            me.refresh(&ed, true);
        });
    }

    /// The answers as they are, asked again when old.
    fn read<T>(editor: &Arc<Editor>, f: impl FnOnce(&Shown) -> T) -> T {
        let me = GamePrefs::of(editor);
        me.refresh(editor, false);
        f(&me.shown.lock())
    }
}

fn text_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn project_open(v: &Value) -> bool {
    v.get("ok").and_then(Value::as_bool) == Some(true)
}

fn own_game(v: &Value) -> Option<String> {
    (v.pointer("/game/source").and_then(Value::as_str) == Some("project"))
        .then(|| v.pointer("/game/root").and_then(Value::as_str).unwrap_or_default().replace('/', "\\"))
}

// ---- Game > Default Game ------------------------------------------------------------------------

fn default_folder(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        let d = text_of(&s.game, "default");
        if d.is_empty() { "No game found".to_owned() } else { d.replace('/', "\\") }
    })
}

fn default_found(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        match text_of(&s.game, "default_found") {
            "preference" => "Picked by you",
            "linked" => "A game this editor was linked to before",
            "installed" => "The installed Dead Air Refined",
            "beside_editor" => "Beside the editor",
            _ => "Not found",
        }
        .to_owned()
    })
}

fn game_version(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        let v = text_of(&s.game, "version");
        if v.is_empty() { "-".to_owned() } else { v.to_owned() }
    })
}

fn change_default(ed: &Arc<Editor>) {
    let start = GamePrefs::read(ed, |s| text_of(&s.game, "default").replace('/', "\\"));
    let e = ed.clone();
    pick_folder(ed, "Default Game", &start, move |folder, _| {
        GamePrefs::of(&e).change(&e, "game.link", args([("path", s(&folder)), ("scope", s("default"))]));
    });
}

fn find_again(ed: &Arc<Editor>) {
    GamePrefs::of(ed).change(ed, "game.link", args([("find", json!(true))]));
}

fn last_error(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| s.error.clone())
}

fn always(_: &Arc<Editor>) -> Enabled {
    Enabled::when(true, "")
}

fn nothing(_: &Arc<Editor>) -> String {
    String::new()
}

const DEFAULT_GAME: [PrefRow; 5] = [
    PrefRow {
        label: "Folder",
        help: "The Dead Air Refined install every project reads unless it names its own. Found by itself: a game this editor knew, the installed game, a game beside the editor.",
        kind: PrefKind::ReadOnly { value: default_folder },
    },
    PrefRow {
        label: "Found",
        help: "How the editor came to this game.",
        kind: PrefKind::ReadOnly { value: default_found },
    },
    PrefRow {
        label: "Version",
        help: "The version of the game's xrEngine.exe.",
        kind: PrefKind::ReadOnly { value: game_version },
    },
    PrefRow {
        label: "Change",
        help: "Picks another game folder for every project that names none.",
        kind: PrefKind::Button { text: "Change...", run: change_default, enabled: always, status: nothing },
    },
    PrefRow {
        label: "Find Again",
        help: "Looks for the game again, as at the first start.",
        kind: PrefKind::Button { text: "Find Again", run: find_again, enabled: always, status: last_error },
    },
];

// ---- Game > This Project --------------------------------------------------------------------------

fn project_game(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        if !project_open(&s.project) {
            return "No project is open".to_owned();
        }
        own_game(&s.project).unwrap_or_else(|| "Uses the default game".to_owned())
    })
}

fn change_project_game(ed: &Arc<Editor>) {
    let start = GamePrefs::read(ed, |s| own_game(&s.project).unwrap_or_default());
    let e = ed.clone();
    pick_folder(ed, "Game of This Project", &start, move |folder, _| {
        GamePrefs::of(&e).change(&e, "game.link", args([("path", s(&folder)), ("scope", s("project"))]));
    });
}

fn use_default_game(ed: &Arc<Editor>) {
    GamePrefs::of(ed).change(ed, "game.link", args([("scope", s("project"))]));
}

fn with_project(ed: &Arc<Editor>) -> Enabled {
    Enabled::when(ed.project_opt().is_some(), "Open a project first.")
}

fn with_own_game(ed: &Arc<Editor>) -> Enabled {
    let own = GamePrefs::read(ed, |s| own_game(&s.project).is_some());
    Enabled::when(own, "The project uses the default game already.")
}

const THIS_PROJECT: [PrefRow; 3] = [
    PrefRow {
        label: "Game",
        help: "A game of the open project's own, on this computer only: the project file names no game.",
        kind: PrefKind::ReadOnly { value: project_game },
    },
    PrefRow {
        label: "Link Game",
        help: "Picks a game folder for the open project alone.",
        kind: PrefKind::Button {
            text: "Change...",
            run: change_project_game,
            enabled: with_project,
            status: nothing,
        },
    },
    PrefRow {
        label: "Use Default Game",
        help: "The open project reads the default game again.",
        kind: PrefKind::Button {
            text: "Use Default Game",
            run: use_default_game,
            enabled: with_own_game,
            status: nothing,
        },
    },
];

// ---- Library ----------------------------------------------------------------------------------------

fn library_folder(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        let f = text_of(&s.project, "library_root");
        if f.is_empty() { gz_project::library_root(ed).display().to_string() } else { f.replace('/', "\\") }
    })
}

fn library_list(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        if !project_open(&s.project) {
            return "No project is open".to_owned();
        }
        match s.project.get("library").and_then(Value::as_array).map_or(0, Vec::len) {
            0 => "Nothing: the project is complete by itself".to_owned(),
            1 => "1 object (File > Project > Collect Library Assets... copies it in)".to_owned(),
            n => format!("{n} objects (File > Project > Collect Library Assets... copies them in)"),
        }
    })
}

fn change_library(ed: &Arc<Editor>) {
    let start = library_folder(ed);
    let e = ed.clone();
    pick_folder(ed, "Library Folder", &start, move |folder, _| {
        GamePrefs::of(&e).change(&e, "project.set_library", args([("path", s(&folder))]));
    });
}

fn editors_library(ed: &Arc<Editor>) {
    GamePrefs::of(ed).change(ed, "project.set_library", args([("path", s(""))]));
}

const LIBRARY: [PrefRow; 4] = [
    PrefRow {
        label: "Folder",
        help: "The editor's library (its rawdata and gamedata): what a scene takes from outside the project. Placing an object from it copies the object into the project.",
        kind: PrefKind::ReadOnly { value: library_folder },
    },
    PrefRow {
        label: "Change",
        help: "Picks another library folder.",
        kind: PrefKind::Button {
            text: "Change...",
            run: change_library,
            enabled: always,
            status: last_error,
        },
    },
    PrefRow {
        label: "Use the Editor's",
        help: "The library beside the editor again.",
        kind: PrefKind::Button {
            text: "Use the Editor's",
            run: editors_library,
            enabled: always,
            status: nothing,
        },
    },
    PrefRow {
        label: "This Project Takes",
        help: "What the project file's library list names: the objects its scenes still take from the library.",
        kind: PrefKind::ReadOnly { value: library_list },
    },
];

// ---- General > Project Files ------------------------------------------------------------------------

fn associate_on(ed: &Arc<Editor>) -> bool {
    GamePrefs::read(ed, |s| s.association.get("on").and_then(Value::as_bool).unwrap_or(true))
}

fn set_associate(ed: &Arc<Editor>, on: bool) {
    GamePrefs::of(ed).change(ed, "project.associate", args([("on", json!(on))]));
}

fn register_now(ed: &Arc<Editor>) {
    GamePrefs::of(ed).change(ed, "project.associate", args([("now", json!(true))]));
}

fn association_status(ed: &Arc<Editor>) -> String {
    GamePrefs::read(ed, |s| {
        let ours = s.association.get("ours").and_then(Value::as_bool) == Some(true);
        match s.association.get("program").and_then(Value::as_str) {
            Some(_) if ours => "Project files open with this editor.".to_owned(),
            Some(p) => format!("Project files open with {}.", p.replace('/', "\\")),
            None => "No program opens project files yet.".to_owned(),
        }
    })
}

const PROJECT_FILES: [PrefRow; 2] = [
    PrefRow {
        label: "Open .grizzly Files with This Editor",
        help: "A double click on a project file (.grizzly) opens the project in this editor. The editor sets it for your Windows account when it starts.",
        kind: PrefKind::Switch { get: associate_on, set: set_associate, default: true },
    },
    PrefRow {
        label: "Register Now",
        help: "Makes project files open with this editor now.",
        kind: PrefKind::Button {
            text: "Register Now",
            run: register_now,
            enabled: always,
            status: association_status,
        },
    },
];

/// Plugs the groups into Preferences.
pub(crate) fn install(b: &mut ShellBuilder) {
    b.pref_group(PrefGroup {
        page: "Game",
        icon: Icon::GameFolder,
        section: "Default Game",
        rows: &DEFAULT_GAME,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Game",
        icon: Icon::GameFolder,
        section: "This Project",
        rows: &THIS_PROJECT,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Library",
        icon: Icon::Folder,
        section: "Library",
        rows: &LIBRARY,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "General",
        icon: Icon::Info,
        section: "Project Files",
        rows: &PROJECT_FILES,
        note: None,
    });
}
