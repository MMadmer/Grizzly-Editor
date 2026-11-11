//! The scene's rows of the C++ command table and the Level tab's commands that open a file
//! dialog or ask first (`UI_LevelMain.cpp` `CLevelMain::RegisterCommands`): New Scene, Open
//! Scene..., Recent Scene, Save As..., the tools by their keys, Cut / Copy / Paste, the selection
//! files, the level parts, the reports (Validate, Summary Info, the compiler's error list) and
//! the level builds behind `ConfirmLevelBuild`.
//!
//! Every action ends in an editor command (`scene_edit`, `selection_file`, `level_parts`,
//! `scene_summary`, `compiler_errors`, `scene_validate`, `make_*` ...): the menus, the keys, the
//! palette, `exec_command` and MCP run the same thing. File dialogs open on a thread of their own.

use std::path::PathBuf;
use std::sync::Arc;

use egui::Context;
use gz_app::{
    CommandVar, CppAction, CppCommand, CppSub, DialogDesc, DocCx, EditorSettings, MenuContent, MenuEntry,
    ShellBuilder,
};
use gz_editor::{Args, Editor, SmolStr, Value, json};
use gz_scene::{ObjClass, SceneCmd, SceneService};
use gz_ui::dialog::ResultSpec;
use gz_ui::theme::Tone;
use gz_ui::{Answer, ConfirmSpec, Dialog, DialogSize, Icon, MenuItem, menu_item};
use parking_lot::Mutex;

use crate::tools::asker;

/// The keys the rows of this file default to (the repository's `level.ini [shortcuts]`).
pub const DEFAULT_KEYS: [(&str, &str); 24] = [
    ("COMMAND_CHANGE_TARGET.\"Object\"", "1"),
    ("COMMAND_CHANGE_TARGET.\"Light\"", "2"),
    ("COMMAND_CHANGE_TARGET.\"Sound Source\"", "3"),
    ("COMMAND_CHANGE_TARGET.\"Sound Env\"", "4"),
    ("COMMAND_CHANGE_TARGET.\"Glow\"", "5"),
    ("COMMAND_CHANGE_TARGET.\"Shape\"", "6"),
    ("COMMAND_CHANGE_TARGET.\"Spawn Point\"", "7"),
    ("COMMAND_CHANGE_TARGET.\"Way\"", "8"),
    ("COMMAND_CHANGE_TARGET.\"Way Point\"", "Ctrl+8"),
    ("COMMAND_CHANGE_TARGET.\"Toggle Way Mode\"", "Ctrl+Q"),
    ("COMMAND_CHANGE_TARGET.\"Sector\"", "0"),
    ("COMMAND_CHANGE_TARGET.\"Portal\"", "9"),
    ("COMMAND_CHANGE_TARGET.\"Group\"", "Ctrl+1"),
    ("COMMAND_CHANGE_TARGET.\"Particle System\"", "Ctrl+2"),
    ("COMMAND_CHANGE_TARGET.\"Detail Objects\"", "Ctrl+3"),
    ("COMMAND_CHANGE_TARGET.\"AI Map\"", "Ctrl+4"),
    ("COMMAND_CHANGE_TARGET.\"Static Wallmark\"", "Ctrl+5"),
    ("COMMAND_TOGGLE_AIMAP_VISIBILITY", "Backslash"),
    ("COMMAND_CLEAR", "Ctrl+N"),
    ("COMMAND_LOAD", "Ctrl+L"),
    ("COMMAND_LOAD_FIRSTRECENT", "Ctrl+R"),
    ("COMMAND_SAVE.\"Save As\"", "Ctrl+Shift+S"),
    ("COMMAND_BUILD", "F9"),
    ("COMMAND_MAKE_GAME", "F5"),
];

/// `CLevelMain::RegisterCommands`'s Change Target: the face, the class, the way mode.
const TARGETS: [(&str, ObjClass, u32); 18] = [
    ("Object", ObjClass::SceneObject, 0),
    ("Light", ObjClass::Light, 0),
    ("Sound Source", ObjClass::SoundSrc, 0),
    ("Sound Env", ObjClass::SoundEnv, 0),
    ("Glow", ObjClass::Glow, 0),
    ("Shape", ObjClass::Shape, 0),
    ("Spawn Point", ObjClass::SpawnPoint, 0),
    ("Way", ObjClass::Way, 0),
    ("Way Point", ObjClass::Way, 1),
    ("Toggle Way Mode", ObjClass::Way, 2),
    ("Sector", ObjClass::Sector, 0),
    ("Portal", ObjClass::Portal, 0),
    ("Group", ObjClass::Group, 0),
    ("Particle System", ObjClass::Particles, 0),
    ("Detail Objects", ObjClass::DetailObjects, 0),
    ("AI Map", ObjClass::AiMap, 0),
    ("Static Wallmark", ObjClass::Wallmark, 0),
    ("Fog Volume", ObjClass::FogVolume, 0),
];

/// A level build of the Build menu and the C++ command table.
struct Make {
    command: &'static str,
    id: u32,
    name: &'static str,
    title: &'static str,
    headline: &'static str,
    consequence: &'static str,
    /// The command table's description.
    desc: &'static str,
    destructive: bool,
}

/// The level builds: their command, C++ id, row and question.
const MAKES: [Make; 6] = [
    Make {
        command: "make_all",
        id: 87,
        name: "COMMAND_BUILD",
        title: "Make All",
        headline: "Empty the level folder and build every level file of this scene into it?",
        consequence: "Everything in the folder is deleted for good first, then built anew.",
        desc: "Compile\\Build",
        destructive: true,
    },
    Make {
        command: "make_game",
        id: 88,
        name: "COMMAND_MAKE_GAME",
        title: "Make Game",
        headline: "Write the game files of this scene into its level folder?",
        consequence: "Files of the same name there are replaced.",
        desc: "Compile\\Make Game",
        destructive: false,
    },
    Make {
        command: "make_details",
        id: 89,
        name: "COMMAND_MAKE_DETAILS",
        title: "Make Details",
        headline: "Export the detail objects of this scene into its level folder?",
        consequence: "The detail objects there are replaced.",
        desc: "Compile\\Make Details",
        destructive: false,
    },
    Make {
        command: "make_hom",
        id: 90,
        name: "COMMAND_MAKE_HOM",
        title: "Make HOM",
        headline: "Build the occlusion model (HOM) of this scene into its level folder?",
        consequence: "The occlusion model there is replaced.",
        desc: "Compile\\Make HOM",
        destructive: false,
    },
    Make {
        command: "make_som",
        id: 91,
        name: "COMMAND_MAKE_SOM",
        title: "Make SOM",
        headline: "Build the sound occlusion model (SOM) of this scene into its level folder?",
        consequence: "The sound occlusion model there is replaced.",
        desc: "Compile\\Make SOM",
        destructive: false,
    },
    Make {
        command: "make_aimap",
        id: 92,
        name: "COMMAND_MAKE_AIMAP",
        title: "Make AI Map",
        headline: "Export the AI map of this scene into its level folder?",
        consequence: "The AI map there is replaced.",
        desc: "Compile\\Make AI Map",
        destructive: false,
    },
];

/// The table row of a level build's command (`make_all` -> `COMMAND_BUILD`).
#[must_use]
pub fn make_id(command: &str) -> Option<&'static str> {
    MAKES.iter().find(|m| m.command == command).map(|m| m.name)
}

fn scene(editor: &Editor) -> Option<Arc<SceneService>> {
    editor.project_opt().and_then(|s| s.services.get::<SceneService>()).filter(|s| s.store.snapshot().valid)
}

fn args(pairs: &[(&str, Value)]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert((*k).into(), v.clone());
    }
    a
}

/// Runs an editor command off the UI thread; its error goes to the notices.
fn run(editor: &Arc<Editor>, command: &'static str, a: Args) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let reply = ed.call(command, a, ed.ui_origin()).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
            ed.notices.error(text.to_owned());
        }
    });
}

/// The system's file dialog on a thread of its own; `then` gets the file.
fn file_dialog(
    editor: &Arc<Editor>,
    title: &'static str,
    filter: (&'static str, &'static [&'static str]),
    dir: PathBuf,
    save: Option<String>,
    then: impl FnOnce(&Arc<Editor>, PathBuf) + Send + 'static,
) {
    let ed = editor.clone();
    let spawned = std::thread::Builder::new().name("gz-ui-level.file_dialog".into()).spawn(move || {
        let _ = std::fs::create_dir_all(&dir);
        let d = rfd::FileDialog::new().set_title(title).add_filter(filter.0, filter.1).set_directory(&dir);
        let picked = match save {
            Some(name) => d.set_file_name(name).save_file(),
            None => d.pick_file(),
        };
        if let Some(f) = picked {
            then(&ed, f);
        }
    });
    if let Err(e) = spawned {
        editor.log.warn(format!("the file dialog did not open: {e}"));
    }
}

fn path_text(p: &std::path::Path) -> Value {
    Value::String(p.display().to_string())
}

// ---- the scene's file -------------------------------------------------------------------------

/// Unsaved scene work: asks Save / Don't Save / Cancel (the kit's `unsaved_prompt`), then `go`
/// runs with the arguments that settle it.
fn settle_then(editor: &Arc<Editor>, ctx: &Context, go: impl FnOnce(&Arc<Editor>, Args) + Send + 'static) {
    let Some(svc) = scene(editor) else {
        go(editor, Args::new());
        return;
    };
    if !svc.store.is_dirty() {
        go(editor, Args::new());
        return;
    }
    let name = svc.store.snapshot().meta.name();
    let what = vec![if name.is_empty() { "Untitled scene".to_owned() } else { name }];
    let ed = editor.clone();
    let line =
        gz_ui::dialog::unsaved_prompt(ctx, asker(editor), "scene", &what, move |answer| match answer {
            Answer::Yes => go(&ed, args(&[("save", Value::Bool(true))])),
            Answer::No => go(&ed, args(&[("discard", Value::Bool(true))])),
            Answer::Cancel => {}
        });
    if let Some(l) = line {
        editor.log.plain(l);
    }
}

/// File > New Scene (`CommandClear`).
pub fn new_scene(editor: &Arc<Editor>, ctx: &Context) {
    settle_then(editor, ctx, |ed, a| run(ed, "scene_new", a));
}

/// Opens a scene file, asking about unsaved work first.
pub fn open_scene_file(editor: &Arc<Editor>, ctx: &Context, file: String) {
    settle_then(editor, ctx, move |ed, mut a| {
        a.insert("scene".into(), Value::String(file));
        // a scene saved on the way is clean: what is left is discarded on purpose
        let ed = ed.clone();
        ed.clone().exec.spawn(async move {
            if a.get("save").and_then(Value::as_bool) == Some(true) {
                let r = ed.call("save_scene", Args::new(), ed.ui_origin()).await;
                if r.get("ok").and_then(Value::as_bool) != Some(true) {
                    ed.notices.error(
                        r.get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("The scene was not saved.")
                            .to_owned(),
                    );
                    return;
                }
            }
            a.remove("save");
            a.insert("discard".into(), Value::Bool(true));
            let r = ed.call("open_scene", a, ed.ui_origin()).await;
            if r.get("ok").and_then(Value::as_bool) != Some(true) {
                ed.notices.error(
                    r.get("error").and_then(Value::as_str).unwrap_or("The scene did not open.").to_owned(),
                );
            }
        });
    });
}

/// File > Open Scene... (`CommandLoad` without a scene): the system's dialog in `$maps$`.
pub fn open_scene_dialog(editor: &Arc<Editor>, ctx: &Context) {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else { return };
    let dir = svc.maps_dir();
    let ctx = ctx.clone();
    file_dialog(editor, "Open Scene", ("Level", &["level"]), dir, None, move |ed, f| {
        open_scene_file(ed, &ctx, f.display().to_string());
    });
}

/// File > Save As... (`CommandSave` with p2 = 1).
pub fn save_as(editor: &Arc<Editor>) {
    let Some(svc) = scene(editor) else { return };
    let snap = svc.store.snapshot();
    let name = snap
        .meta
        .file
        .as_ref()
        .and_then(|f| f.file_name())
        .map_or_else(|| "level.level".to_owned(), |n| n.to_string_lossy().into_owned());
    file_dialog(editor, "Save Scene As", ("Level", &["level"]), svc.maps_dir(), Some(name), |ed, f| {
        run(ed, "save_scene", args(&[("file", path_text(&f))]));
    });
}

/// File > Recent Scene's list (`scene_recent_list`).
#[must_use]
pub fn recent(editor: &Editor) -> Vec<String> {
    EditorSettings::of(editor).recent_scenes()
}

// ---- Edit and Scene -----------------------------------------------------------------------------

/// Edit > Cut / Copy / Paste / Duplicate: `scene_edit` on the current tool's class.
pub fn edit(editor: &Arc<Editor>, action: &'static str) {
    run(editor, "scene_edit", args(&[("action", Value::String(action.into()))]));
}

/// Scene > Open Selection... (`CommandLoadSelection`).
pub fn open_selection(editor: &Arc<Editor>) {
    let Some(svc) = scene(editor) else { return };
    file_dialog(editor, "Open Selection", ("Level", &["level"]), svc.maps_dir(), None, |ed, f| {
        run(ed, "selection_file", args(&[("action", Value::String("open".into())), ("file", path_text(&f))]));
    });
}

/// Scene > Save Selection As... (`CommandSaveSelection`).
pub fn save_selection(editor: &Arc<Editor>) {
    let Some(svc) = scene(editor) else { return };
    file_dialog(
        editor,
        "Save Selection As",
        ("Level", &["level"]),
        svc.maps_dir(),
        Some("selection.level".into()),
        |ed, f| {
            run(
                ed,
                "selection_file",
                args(&[("action", Value::String("save".into())), ("file", path_text(&f))]),
            );
        },
    );
}

/// Scene > Export Scene / Selection as OBJ... (`EScene::ExportObj`: the name asked in `$import$`).
pub fn export_obj(editor: &Arc<Editor>, selected: bool) {
    let Some(root) = editor.project_root() else { return };
    file_dialog(
        editor,
        "Export as OBJ",
        ("Wavefront OBJ", &["obj"]),
        root.join("import"),
        Some("scene.obj".into()),
        move |ed, f| {
            run(
                ed,
                "scene_export_obj",
                args(&[("file", path_text(&f)), ("selected", Value::Bool(selected))]),
            );
        },
    );
}

/// `CommandMultiRenameObjects`: asks, naming the objects.
pub fn rename_objects(editor: &Arc<Editor>, ctx: &Context) {
    let Some(svc) = scene(editor) else { return };
    let snap = svc.store.snapshot();
    let class = snap.tools.target;
    let sel: Vec<_> = snap.of_class(class).filter(|o| snap.is_selected(o.id)).collect();
    if sel.is_empty() {
        editor.log.info("Nothing is selected: no object is renamed.");
        return;
    }
    let spec = ConfirmSpec {
        id: "rename_objects".into(),
        title: "Rename Objects".into(),
        headline: format!("Rename the {} selected object(s) after their references?", sel.len()),
        items: sel.iter().take(10).map(|o| o.name.to_string()).collect(),
        total: sel.len(),
        consequence: "Each one takes the next free name made from its reference. Undo puts the names back."
            .into(),
        action: "Rename".into(),
        destructive: false,
    };
    let ed = editor.clone();
    let line = gz_ui::dialog::confirm(ctx, asker(editor), spec, move |a| {
        if a == Answer::Yes {
            run(&ed, "rename_objects", Args::new());
        }
    });
    if let Some(l) = line {
        editor.log.plain(l);
    }
}

/// Scene > Validate (`CommandValidateScene`): the findings in the kit's Result dialog.
pub fn validate(editor: &Arc<Editor>, ctx: &Context) {
    let ed = editor.clone();
    let ctx = ctx.clone();
    editor.exec.spawn(async move {
        let r = ed.call("scene_validate", Args::new(), ed.ui_origin()).await;
        let ok = r.get("ok").and_then(Value::as_bool) == Some(true);
        let passed = ok && r.get("ok").is_some() && r.get("lines").is_some() && {
            // the reply's own `ok` is the command's; the validation's is the last line
            r.get("lines").and_then(Value::as_array).and_then(|l| l.last()).and_then(Value::as_str)
                == Some("Validation OK!")
        };
        let lines: Vec<String> = r
            .get("lines")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let (tone, headline) = if !ok {
            (
                Tone::Error,
                r.get("error").and_then(Value::as_str).unwrap_or("The scene was not validated.").to_owned(),
            )
        } else if passed {
            (Tone::Success, "Validation OK!".to_owned())
        } else {
            (Tone::Error, "Validation FAILED!".to_owned())
        };
        let mut spec = ResultSpec::new("validate_result", "Validate", tone, &headline);
        spec.warnings = lines.into_iter().filter(|l| !l.starts_with("Validation ")).collect();
        let line = gz_ui::dialog::result(&ctx, asker(&ed), spec);
        if let Some(l) = line {
            ed.log.plain(l);
        }
        ctx.request_repaint();
    });
}

/// The Summary Info sheet's rows (`SSceneSummary::FillProp`), shown in `dlg.scene_summary`.
#[derive(Default)]
struct SummarySheet {
    open: bool,
    rows: Vec<(String, String)>,
    busy: bool,
}

fn sheet(editor: &Editor) -> Arc<Mutex<SummarySheet>> {
    editor.services.get_or_init(|| Mutex::new(SummarySheet::default()))
}

/// Scene > Summary Info: clear, collect, show (`CommandClearSceneSummary` ... `Show`).
pub fn summary(editor: &Arc<Editor>) {
    let s = sheet(editor);
    {
        let mut st = s.lock();
        st.open = true;
        st.busy = true;
        st.rows.clear();
    }
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let _ = ed
            .call("scene_summary", args(&[("action", Value::String("clear".into()))]), ed.ui_origin())
            .await;
        let r = ed
            .call("scene_summary", args(&[("action", Value::String("collect".into()))]), ed.ui_origin())
            .await;
        let rows: Vec<(String, String)> = r
            .get("rows")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|row| {
                        let k = row.get("key").and_then(Value::as_str).unwrap_or("").to_owned();
                        let v = row.get("value").and_then(Value::as_str).unwrap_or("").to_owned();
                        (k, v)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let s = sheet(&ed);
        let mut st = s.lock();
        st.rows = rows;
        st.busy = false;
        drop(st);
        ed.exec.wake().ping();
    });
}

fn draw_summary(ui: &mut egui::Ui, editor: &Arc<Editor>) {
    let s = sheet(editor);
    let (mut open, rows, busy) = {
        let st = s.lock();
        (st.open, st.rows.clone(), st.busy)
    };
    if !open {
        return;
    }
    let mut export = false;
    let mut close = false;
    Dialog::new("Summary Info", "dlg.scene_summary", DialogSize::L).show(
        ui,
        &mut open,
        |ui| {
            if busy {
                gz_ui::state::empty_state(
                    ui,
                    Icon::Info,
                    "Collecting",
                    "The scene's tools report what they hold.",
                    None,
                );
                return;
            }
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                gz_ui::PropertyGrid::new("summary").show(ui, |g| {
                    for (k, v) in &rows {
                        g.row(k, "", gz_ui::PropFlags::NONE, 0, |ui| {
                            gz_ui::text::label(
                                ui,
                                v,
                                gz_ui::Font::Body,
                                gz_ui::palette(ui.ctx()).text,
                                gz_ui::Clip::End,
                            );
                        });
                    }
                });
            });
        },
        |f| {
            if f.button("Export...", gz_ui::Kind::Secondary, false, !busy, "Write the summary as a text file")
            {
                export = true;
            }
            if f.button("Close", gz_ui::Kind::Primary, true, true, "") {
                close = true;
            }
        },
    );
    if export {
        let level =
            scene(editor).map(|s| s.store.snapshot().meta.level.level_path.to_string()).unwrap_or_default();
        if let Some(root) = editor.project_root() {
            file_dialog(
                editor,
                "Export Summary",
                ("Text", &["txt"]),
                root.join("import"),
                Some(format!("{level}.txt")),
                |ed, f| {
                    run(
                        ed,
                        "scene_summary",
                        args(&[("action", Value::String("export".into())), ("file", path_text(&f))]),
                    );
                },
            );
        }
    }
    s.lock().open = open && !close;
}

/// Build > Import / Export / Clear Error List and Scene > Clear Debug Draw.
pub fn import_errors(editor: &Arc<Editor>) {
    let Some(root) = editor.project_root() else { return };
    file_dialog(
        editor,
        "Import Error List",
        ("Error list", &["err", "*"]),
        root.join("logs"),
        None,
        |ed, f| {
            run(
                ed,
                "compiler_errors",
                args(&[("action", Value::String("import".into())), ("file", path_text(&f))]),
            );
        },
    );
}

pub fn export_errors(editor: &Arc<Editor>) {
    let Some(root) = editor.project_root() else { return };
    file_dialog(
        editor,
        "Export Error List",
        ("Error list", &["err"]),
        root.join("logs"),
        Some("build.err".into()),
        |ed, f| {
            run(
                ed,
                "compiler_errors",
                args(&[("action", Value::String("export".into())), ("file", path_text(&f))]),
            );
        },
    );
}

pub fn clear_debug_draw(editor: &Arc<Editor>) {
    run(editor, "compiler_errors", args(&[("action", Value::String("clear".into()))]));
}

/// `ConfirmLevelBuild`: the level folder named, then the build. A scene that names no folder is
/// not asked about - the build stops with its own error.
pub fn ask_make(editor: &Arc<Editor>, ctx: &Context, command: &'static str) {
    let Some(m) = MAKES.iter().find(|m| m.command == command) else { return };
    let level =
        scene(editor).map(|s| s.store.snapshot().meta.level.level_path.to_string()).unwrap_or_default();
    if level.is_empty() {
        run(editor, command, Args::new());
        return;
    }
    // `$game_levels$\<level>`: where the build writes
    let folder = editor.paths.root.join("gamedata").join("levels").join(&level);
    let spec = ConfirmSpec {
        id: m.command.into(),
        title: m.title.into(),
        headline: m.headline.into(),
        items: vec![folder.display().to_string()],
        total: 1,
        consequence: m.consequence.into(),
        action: m.title.into(),
        destructive: m.destructive,
    };
    let ed = editor.clone();
    let line = gz_ui::dialog::confirm(ctx, asker(editor), spec, move |a| {
        if a == Answer::Yes {
            run(&ed, command, Args::new());
        }
    });
    if let Some(l) = line {
        editor.log.plain(l);
    }
}

// ---- the C++ rows ----------------------------------------------------------------------------------

fn ok() -> Result<CommandVar, String> {
    Ok(CommandVar::Int(1))
}

fn has_scene(ed: &Editor, _: &str) -> bool {
    scene(ed).is_some()
}

fn need_scene(cx: &DocCx<'_>) -> Result<Arc<SceneService>, String> {
    scene(cx.editor).ok_or_else(|| "no scene".to_owned())
}

/// The table's rows of the scene, the File menu's rows and the Summary Info sheet.
pub(crate) fn install(b: &mut ShellBuilder) {
    // Change Target: the tool of a class, its keys
    let subs: Vec<CppSub> = TARGETS
        .iter()
        .map(|(name, class, sub)| {
            let id = format!("COMMAND_CHANGE_TARGET.\"{name}\"");
            let keys = DEFAULT_KEYS.iter().find(|(i, _)| *i == id).map_or("", |(_, k)| *k);
            CppSub::ints(name, class.id(), *sub).keys(keys)
        })
        .collect();
    b.cpp_command(
        CppCommand::new(
            57,
            "COMMAND_CHANGE_TARGET",
            "Change Target",
            CppAction::shell(|cx, p1, p2| {
                let svc = need_scene(cx)?;
                let class = p1.int().and_then(ObjClass::from_id).ok_or("no such class")?;
                // `CommandChangeTarget`: a part that is not loaded has no tool
                if !svc.store.snapshot().tools.part(class).enabled {
                    return Ok(CommandVar::Int(0));
                }
                svc.store.post(SceneCmd::SetTarget(class), cx.editor.ui_origin());
                if class == ObjClass::Way {
                    let now = svc.store.snapshot().tools.obj.way_points;
                    let points = match p2.int() {
                        Some(1) => Some(true),
                        Some(2) => Some(!now),
                        Some(0) => Some(false),
                        _ => None,
                    };
                    if let Some(p) = points {
                        let mode = if p { "points" } else { "ways" };
                        run(cx.editor, "way_tool", args(&[("action", json!("mode")), ("mode", json!(mode))]));
                    }
                }
                ok()
            }),
        )
        .subs(subs)
        .face("Tools", "Tool", Icon::None)
        .enabled(has_scene),
    );
    // the parts: Enable, Read Only, Show (`p1` class, `p2` on)
    for (id, name, key) in
        [(58, "COMMAND_ENABLE_TARGET", "enabled"), (59, "COMMAND_READONLY_TARGET", "readonly")]
    {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                "",
                CppAction::editor("level_parts", move |p1, p2| {
                    let class = p1.int().and_then(ObjClass::from_id).ok_or("p1 names the class by its id")?;
                    let mut a = args(&[("class", json!(class.name())), (key, json!(p2.truthy()))]);
                    // the editor's own run unloads only what is saved (`IfModified` asked first)
                    a.insert("save".into(), json!(true));
                    Ok(a)
                }),
            )
            .editable(false)
            .enabled(has_scene),
        );
    }
    b.cpp_command(
        CppCommand::new(
            60,
            "COMMAND_SHOW_TARGET",
            "",
            CppAction::shell(|cx, p1, p2| {
                let svc = need_scene(cx)?;
                let class = p1.int().and_then(ObjClass::from_id).ok_or("no such class")?;
                svc.store.post(
                    SceneCmd::ToolFlags { class, visible: Some(p2.truthy()), readonly: None },
                    cx.editor.ui_origin(),
                );
                ok()
            }),
        )
        .editable(false)
        .enabled(has_scene),
    );
    for (id, name, part) in [(79, "COMMAND_LOAD_LEVEL_PART", true), (80, "COMMAND_UNLOAD_LEVEL_PART", false)]
    {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                "",
                CppAction::editor("level_parts", move |p1, _| {
                    let class = p1.int().and_then(ObjClass::from_id).ok_or("p1 names the class by its id")?;
                    Ok(args(&[
                        ("class", json!(class.name())),
                        ("enabled", json!(part)),
                        ("discard", json!(true)),
                    ]))
                }),
            )
            .editable(false)
            .enabled(has_scene),
        );
    }
    b.cpp_command(
        CppCommand::new(
            62,
            "COMMAND_MULTI_RENAME_OBJECTS",
            "",
            CppAction::shell(|cx, _, p2| {
                // `CommandConfirmed(p2)`: a confirmed call asks nothing
                if p2.truthy() {
                    run(cx.editor, "rename_objects", Args::new());
                } else {
                    rename_objects(cx.editor, cx.ctx);
                }
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Rename Objects After References", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            44,
            "COMMAND_TOGGLE_AIMAP_VISIBILITY",
            "Visibility\\Toggle AIMap",
            CppAction::shell(|cx, _, _| {
                gz_ui_viewport::Viewport::of(cx.editor).with(|s| s.view.ai_map_shown = !s.view.ai_map_shown);
                cx.editor.exec.wake().ping();
                ok()
            }),
        )
        .keys("Backslash")
        .face("View", "Show AI Map", Icon::None),
    );
    // the scene's file
    b.cpp_command(
        CppCommand::new(
            39,
            "COMMAND_CLEAR",
            "File\\Clear Scene",
            CppAction::shell(|cx, _, _| {
                new_scene(cx.editor, cx.ctx);
                ok()
            }),
        )
        .keys("Ctrl+N")
        .face("File", "New Scene", Icon::None)
        .enabled(|ed, _| ed.project_opt().is_some()),
    );
    b.cpp_command(
        CppCommand::new(
            40,
            "COMMAND_LOAD",
            "File\\Load Level",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(file) => open_scene_file(cx.editor, cx.ctx, file.to_owned()),
                    None => open_scene_dialog(cx.editor, cx.ctx),
                }
                ok()
            }),
        )
        .keys("Ctrl+L")
        .face("File", "Open Scene...", Icon::Open)
        .enabled(|ed, _| ed.project_opt().is_some()),
    );
    b.cpp_command(
        CppCommand::new(
            41,
            "COMMAND_SAVE",
            "File",
            CppAction::shell(|cx, p1, p2| {
                if p2.int() == Some(1) {
                    save_as(cx.editor);
                    return ok();
                }
                let file = p1.text().filter(|f| !f.is_empty()).map(str::to_owned);
                match file {
                    Some(f) => run(cx.editor, "save_scene", args(&[("file", json!(f))])),
                    None => match scene(cx.editor).and_then(|s| s.store.snapshot().meta.file.clone()) {
                        Some(_) => run(cx.editor, "save_scene", Args::new()),
                        None => save_as(cx.editor),
                    },
                }
                ok()
            }),
        )
        .subs(vec![
            CppSub::ints("Save", 0, 0).label("Save Scene"),
            CppSub::ints("Save As", 0, 1).label("Save Scene As...").keys("Ctrl+Shift+S"),
        ])
        .face("File", "Save Scene", Icon::Save)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            106,
            "COMMAND_LOAD_FIRSTRECENT",
            "File\\Load First Recent",
            CppAction::shell(|cx, _, _| match recent(cx.editor).into_iter().next() {
                Some(f) => {
                    open_scene_file(cx.editor, cx.ctx, f);
                    ok()
                }
                None => Ok(CommandVar::Int(0)),
            }),
        )
        .keys("Ctrl+R")
        .face("File", "Open Last Scene", Icon::None)
        .enabled(|ed, _| ed.project_opt().is_some() && !EditorSettings::of(ed).recent_scenes().is_empty()),
    );
    // Edit
    for (id, name, desc, action, label) in [
        (74, "COMMAND_CUT", "Edit\\Cut", "cut", "Cut"),
        (75, "COMMAND_COPY", "Edit\\Copy", "copy", "Copy"),
        (76, "COMMAND_PASTE", "Edit\\Paste", "paste", "Paste"),
    ] {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                desc,
                CppAction::shell(move |cx, _, _| {
                    edit(cx.editor, action);
                    ok()
                }),
            )
            .face("Edit", label, Icon::None)
            .enabled(|ed, tab| tab == crate::LEVEL && has_scene(ed, tab)),
        );
    }
    // the selection files, the reports
    b.cpp_command(
        CppCommand::new(
            77,
            "COMMAND_LOAD_SELECTION",
            "",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(f) => run(
                        cx.editor,
                        "selection_file",
                        args(&[("action", json!("open")), ("file", json!(f))]),
                    ),
                    None => open_selection(cx.editor),
                }
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Open Selection...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            78,
            "COMMAND_SAVE_SELECTION",
            "",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(f) => run(
                        cx.editor,
                        "selection_file",
                        args(&[("action", json!("save")), ("file", json!(f))]),
                    ),
                    None => save_selection(cx.editor),
                }
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Save Selection As...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            69,
            "COMMAND_CLEAR_DEBUG_DRAW",
            "",
            CppAction::editor("compiler_errors", |_, _| Ok(args(&[("action", json!("clear"))]))),
        )
        .editable(false)
        .face("Scene", "Clear Debug Draw", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            70,
            "COMMAND_IMPORT_COMPILER_ERROR",
            "",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(f) => run(
                        cx.editor,
                        "compiler_errors",
                        args(&[("action", json!("import")), ("file", json!(f))]),
                    ),
                    None => import_errors(cx.editor),
                }
                ok()
            }),
        )
        .editable(false)
        .face("Build", "Import Error List...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            71,
            "COMMAND_EXPORT_COMPILER_ERROR",
            "",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(f) => run(
                        cx.editor,
                        "compiler_errors",
                        args(&[("action", json!("export")), ("file", json!(f))]),
                    ),
                    None => export_errors(cx.editor),
                }
                ok()
            }),
        )
        .editable(false)
        .face("Build", "Export Error List...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            72,
            "COMMAND_VALIDATE_SCENE",
            "",
            CppAction::shell(|cx, _, _| {
                validate(cx.editor, cx.ctx);
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Validate", Icon::None)
        .enabled(has_scene),
    );
    for (id, name, action) in
        [(81, "COMMAND_CLEAR_SCENE_SUMMARY", "clear"), (82, "COMMAND_COLLECT_SCENE_SUMMARY", "collect")]
    {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                "",
                CppAction::editor("scene_summary", move |_, _| Ok(args(&[("action", json!(action))]))),
            )
            .editable(false)
            .enabled(has_scene),
        );
    }
    b.cpp_command(
        CppCommand::new(
            83,
            "COMMAND_SHOW_SCENE_SUMMARY",
            "",
            CppAction::shell(|cx, _, _| {
                summary(cx.editor);
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Summary Info", Icon::Info)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            84,
            "COMMAND_EXPORT_SCENE_SUMMARY",
            "",
            CppAction::editor("scene_summary", |p1, _| {
                let f = p1.text().ok_or("p1s names the file")?;
                Ok(args(&[("action", json!("export")), ("file", json!(f))]))
            }),
        )
        .editable(false)
        .enabled(has_scene),
    );
    // the level builds behind `ConfirmLevelBuild` (p2 confirmed asks nothing)
    for Make { command, id, name, title, desc, .. } in MAKES {
        let keys = DEFAULT_KEYS.iter().find(|(i, _)| *i == name).map_or("", |(_, k)| *k);
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                desc,
                CppAction::shell(move |cx, _, p2| {
                    if p2.truthy() {
                        run(cx.editor, command, Args::new());
                    } else {
                        ask_make(cx.editor, cx.ctx, command);
                    }
                    ok()
                }),
            )
            .keys(keys)
            .face("Build", &format!("{title}..."), Icon::None)
            .enabled(|ed, _| has_scene(ed, "") && ed.registry().has("make_all")),
        );
    }
    // the snap list
    for (id, name, action) in [
        (100, "COMMAND_SET_SNAP_OBJECTS", "set"),
        (101, "COMMAND_ADD_SEL_SNAP_OBJECTS", "add"),
        (102, "COMMAND_DEL_SEL_SNAP_OBJECTS", "remove"),
        (103, "COMMAND_CLEAR_SNAP_OBJECTS", "clear"),
        (104, "COMMAND_SELECT_SNAP_OBJECTS", "select"),
    ] {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                "",
                CppAction::editor("snap_objects", move |_, _| Ok(args(&[("action", json!(action))]))),
            )
            .editable(false)
            .enabled(has_scene),
        );
    }
    // File menu: New Scene, Open Scene..., Recent Scene above Save; Save As... below it
    b.menu_entry(MenuEntry {
        menu: SmolStr::new("File"),
        submenu: None,
        group: -10,
        order: 0,
        content: MenuContent::Custom(Box::new(|ui, cx| file_rows(ui, cx, true))),
    });
    b.menu_entry(MenuEntry {
        menu: SmolStr::new("File"),
        submenu: None,
        group: 1,
        order: 50,
        content: MenuContent::Custom(Box::new(|ui, cx| file_rows(ui, cx, false))),
    });
    b.dialog(DialogDesc {
        id: SmolStr::new("scene_summary"),
        window: SmolStr::new("Summary Info###dlg.scene_summary"),
        needs_project: true,
        open: Box::new(|cx| {
            summary(cx.editor);
            Ok(())
        }),
        draw: Box::new(|ui, cx| draw_summary(ui, cx.editor)),
    });
}

/// Marks File > Recent Scene's hook as registered.
struct RecentHooked;

/// File > Recent Scene follows every load and save (`AppendRecentFile`, the path lower-cased).
pub(crate) fn hook_recent(editor: &Editor) {
    editor.services.get_or_init(|| {
        gz_scene::SceneFileHooks::of(editor).on_file(|ed, file| {
            let settings = EditorSettings::of(ed);
            settings.add_recent_scene(&file.display().to_string().to_lowercase());
            settings.save_soon(ed);
        });
        // a scene the project opened before the Level tab was made counts too
        if let Some(svc) = scene(editor)
            && let Some(file) = svc.store.snapshot().meta.file.clone()
        {
            EditorSettings::of(editor).add_recent_scene(&file.display().to_string().to_lowercase());
        }
        RecentHooked
    });
}

fn file_rows(ui: &mut egui::Ui, cx: &mut DocCx<'_>, top: bool) {
    let editor = cx.editor.clone();
    let has_project = editor.project_opt().is_some();
    let keys = |id: &str, d: &str| crate::menus::keys_now(&editor, id, d);
    if top {
        if menu_item(
            ui,
            "New Scene",
            MenuItem { keys: &keys("COMMAND_CLEAR", "Ctrl+N"), enabled: has_project, ..MenuItem::default() },
        ) {
            cx.requests.activate(crate::LEVEL);
            new_scene(&editor, cx.ctx);
        }
        if menu_item(
            ui,
            "Open Scene...",
            MenuItem { keys: &keys("COMMAND_LOAD", "Ctrl+L"), enabled: has_project, ..MenuItem::default() },
        ) {
            cx.requests.activate(crate::LEVEL);
            open_scene_dialog(&editor, cx.ctx);
        }
        let list = recent(&editor);
        gz_app::chrome::local_submenu(
            ui,
            "Recent Scene",
            Icon::None,
            has_project && !list.is_empty(),
            |ui| {
                for (i, f) in list.iter().enumerate() {
                    let k = if i == 0 { keys("COMMAND_LOAD_FIRSTRECENT", "Ctrl+R") } else { String::new() };
                    if menu_item(ui, f, MenuItem { keys: &k, ..MenuItem::default() }) {
                        cx.requests.activate(crate::LEVEL);
                        open_scene_file(&editor, cx.ctx, f.clone());
                    }
                }
            },
        );
    } else {
        let level = cx.active_tab == crate::LEVEL;
        if menu_item(
            ui,
            "Save As...",
            MenuItem {
                keys: &keys("COMMAND_SAVE.\"Save As\"", "Ctrl+Shift+S"),
                enabled: level && scene(&editor).is_some(),
                ..MenuItem::default()
            },
        ) {
            save_as(&editor);
        }
    }
}
