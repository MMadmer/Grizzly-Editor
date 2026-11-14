//! The scene's rows of the command table and the Level tab's commands that open a file
//! dialog or ask first: New Scene, Open
//! Scene..., Recent Scene, Save As..., the tools by their keys, Cut / Copy / Paste, the groups of
//! the project and the reports (Validate, Summary Info, the build's problems).
//!
//! Every action ends in an editor command (`scene.copy`, `scene.group`, `scene.info`,
//! `level.problems`, `level.validate`, `level.export` ...): the menus, the keys,
//! the palette, `exec_command` and MCP run the same thing. File dialogs open on a thread of their
//! own.

use std::path::PathBuf;
use std::sync::Arc;

use egui::Context;
use gz_app::{
    CommandVar, CppAction, CppCommand, CppSub, DialogDesc, DocCx, MenuContent, MenuEntry, ShellBuilder,
};
use gz_editor::{Args, Editor, SmolStr, Value, json};
use gz_scene::{ObjClass, SceneCmd, SceneService};
use gz_ui::dialog::ResultSpec;
use gz_ui::file_dialog::FileRequest;
use gz_ui::theme::Tone;
use gz_ui::{Answer, Dialog, DialogSize, Icon, MenuItem, menu_item};
use parking_lot::Mutex;

use crate::tools::asker;

/// The keys the rows of this file default to (the repository's `level.ini [shortcuts]`).
pub const DEFAULT_KEYS: [(&str, &str); 22] = [
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
];

/// Change Target: the face, the class, the way mode.
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

/// The system's file dialog (the kit's, on a thread of its own); `then` gets the file. For a
/// caller who cannot answer it nothing opens and the row refuses with "name the file".
fn file_dialog(
    editor: &Arc<Editor>,
    ctx: &Context,
    title: &'static str,
    filter: (&'static str, &'static [&'static str]),
    dir: PathBuf,
    save: Option<String>,
    then: impl FnOnce(&Arc<Editor>, PathBuf) + Send + 'static,
) -> Result<(), String> {
    let request = match save {
        Some(name) => FileRequest::save(title, &name),
        None => FileRequest::open(title),
    }
    .filter(filter.0, filter.1)
    .in_folder(dir)
    .creating_folder();
    let ed = editor.clone();
    gz_ui::file_dialog::pick(Some(ctx), asker(editor), request, move |f| then(&ed, f)).map_err(|line| {
        editor.log.plain(line);
        gz_ui::file_dialog::NAME_THE_FILE.to_owned()
    })
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
    settle_then(editor, ctx, |ed, a| run(ed, "scene.new", a));
}

/// Opens a scene file, asking about unsaved work first.
pub fn open_scene_file(editor: &Arc<Editor>, ctx: &Context, file: String) {
    settle_then(editor, ctx, move |ed, mut a| {
        a.insert("path".into(), Value::String(file));
        // a scene saved on the way is clean: what is left is discarded on purpose
        let ed = ed.clone();
        ed.clone().exec.spawn(async move {
            if a.get("save").and_then(Value::as_bool) == Some(true) {
                let r = ed.call("scene.save", Args::new(), ed.ui_origin()).await;
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
            let r = ed.call("scene.open", a, ed.ui_origin()).await;
            if r.get("ok").and_then(Value::as_bool) != Some(true) {
                ed.notices.error(
                    r.get("error").and_then(Value::as_str).unwrap_or("The scene did not open.").to_owned(),
                );
            }
        });
    });
}

/// What `COMMAND_LOAD` without a scene answers a caller no dialog opens for (MCP, `-nodlg`).
pub const NAME_THE_SCENE: &str =
    "p1s names the scene to open (COMMAND_LOAD p1s=<file>): no Open Scene dialog opens for this caller";
/// What `COMMAND_SAVE`'s Save As, or a Save of a scene without a file, answers such a caller.
pub const NAME_THE_SAVE: &str = "p1s names the file to save the scene as (COMMAND_SAVE p1s=<file>): no Save Scene As dialog opens for this caller";

/// File > Open Scene...: the system's dialog in the project (a `.scene`, or an X-Ray SDK
/// `.level` to import).
pub fn open_scene_dialog(editor: &Arc<Editor>, ctx: &Context) -> Result<(), String> {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else {
        return Ok(());
    };
    let dir = svc.root.clone();
    let cx = ctx.clone();
    file_dialog(editor, ctx, "Open Scene", ("Scene", &["scene", "level"]), dir, None, move |ed, f| {
        open_scene_file(ed, &cx, f.display().to_string());
    })
}

/// File > Save As... (`CommandSave` with p2 = 1).
pub fn save_as(editor: &Arc<Editor>, ctx: &Context) -> Result<(), String> {
    let Some(svc) = scene(editor) else { return Ok(()) };
    let snap = svc.store.snapshot();
    let name = snap
        .meta
        .file
        .as_ref()
        .and_then(|f| f.file_name())
        .map_or_else(|| "level.scene".to_owned(), |n| n.to_string_lossy().into_owned());
    file_dialog(editor, ctx, "Save Scene As", ("Scene", &["scene"]), svc.root.clone(), Some(name), |ed, f| {
        run(ed, "scene.save", args(&[("path", path_text(&f))]));
    })
}

/// File > Recent Scene's list: the project's scenes opened last on this machine, project-relative.
#[must_use]
pub fn recent(editor: &Editor) -> Vec<String> {
    gz_project::Project::of(editor).map(|(_, p)| p.state().recent_scenes()).unwrap_or_default()
}

// ---- Edit and Scene -----------------------------------------------------------------------------

/// Edit > Cut / Copy / Paste / Duplicate of the selection, whatever kinds it holds; Cut is a
/// copy, then the delete.
pub fn edit(editor: &Arc<Editor>, action: &'static str) {
    match action {
        "copy" => run(editor, "scene.copy", Args::new()),
        "paste" => run(editor, "scene.paste", Args::new()),
        "duplicate" => run(editor, "scene.duplicate", Args::new()),
        _ => {
            let ed = editor.clone();
            editor.exec.spawn(async move {
                let r = ed.call("scene.copy", Args::new(), ed.ui_origin()).await;
                let r = if r.get("ok").and_then(Value::as_bool) == Some(true) {
                    ed.call("scene.delete", Args::new(), ed.ui_origin()).await
                } else {
                    r
                };
                if r.get("ok").and_then(Value::as_bool) != Some(true) {
                    let text = r.get("error").and_then(Value::as_str).unwrap_or("The cut failed.");
                    ed.notices.error(text.to_owned());
                }
            });
        }
    }
}

/// Scene > Place Group...: a `.group` of the project placed in front of the camera.
pub fn place_group(editor: &Arc<Editor>, ctx: &Context) -> Result<(), String> {
    let Some(svc) = scene(editor) else { return Ok(()) };
    file_dialog(editor, ctx, "Place Group", ("Group", &["group"]), svc.root.clone(), None, |ed, f| {
        run(ed, "scene.place", args(&[("kind", Value::String("group".into())), ("ref", path_text(&f))]));
    })
}

/// Scene > Save Selection as Group...: the selection as a `.group` of the project.
pub fn save_group(editor: &Arc<Editor>, ctx: &Context) -> Result<(), String> {
    let Some(svc) = scene(editor) else { return Ok(()) };
    file_dialog(
        editor,
        ctx,
        "Save Selection as Group",
        ("Group", &["group"]),
        svc.root.join("groups"),
        Some("selection.group".into()),
        |ed, f| {
            run(
                ed,
                "scene.group",
                args(&[("action", Value::String("save".into())), ("path", path_text(&f))]),
            );
        },
    )
}

/// Scene > Export Scene / Selection as OBJ... (the name asked in `$import$`).
pub fn export_obj(editor: &Arc<Editor>, ctx: &Context, selected: bool) -> Result<(), String> {
    let Some(root) = editor.project_root() else { return Ok(()) };
    file_dialog(
        editor,
        ctx,
        "Export as OBJ",
        ("Wavefront OBJ", &["obj"]),
        root.join("import"),
        Some("scene.obj".into()),
        move |ed, f| {
            run(
                ed,
                "level.export",
                args(&[
                    ("kind", Value::String("obj".into())),
                    ("path", path_text(&f)),
                    ("all", Value::Bool(!selected)),
                ]),
            );
        },
    )
}

/// Scene > Validate: the scene checked as a build checks it, the findings in the kit's Result
/// dialog.
pub fn validate(editor: &Arc<Editor>, ctx: &Context) {
    let ed = editor.clone();
    let ctx = ctx.clone();
    editor.exec.spawn(async move {
        let r = ed.call("level.validate", Args::new(), ed.ui_origin()).await;
        let ok = r.get("ok").and_then(Value::as_bool) == Some(true);
        let valid = r.get("valid").and_then(Value::as_bool) == Some(true);
        let lines: Vec<String> = r
            .get("problems")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).map(str::to_owned).collect()
            })
            .unwrap_or_default();
        let (tone, headline) = if !ok {
            (
                Tone::Error,
                r.get("error").and_then(Value::as_str).unwrap_or("The scene was not checked.").to_owned(),
            )
        } else if valid {
            (
                if lines.is_empty() { Tone::Success } else { Tone::Warning },
                "The scene passes its checks.".to_owned(),
            )
        } else {
            (Tone::Error, "The scene does not pass its checks.".to_owned())
        };
        let mut spec = ResultSpec::new("validate_result", "Validate", tone, &headline);
        spec.warnings = lines;
        let line = gz_ui::dialog::result(&ctx, asker(&ed), spec);
        if let Some(l) = line {
            ed.log.plain(l);
        }
        ctx.request_repaint();
    });
}

/// The Summary Info sheet's rows, shown in `dlg.scene_summary`.
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
        let r = ed.call("scene.info", args(&[("summary", Value::Bool(true))]), ed.ui_origin()).await;
        let rows: Vec<(String, String)> = r
            .get("summary")
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
            let _ = file_dialog(
                editor,
                ui.ctx(),
                "Export Summary",
                ("Text", &["txt"]),
                root.join("import"),
                Some(format!("{level}.txt")),
                |ed, f| {
                    run(ed, "scene.info", args(&[("summary", Value::Bool(true)), ("path", path_text(&f))]));
                },
            );
        }
    }
    s.lock().open = open && !close;
}

/// Scene > Clear Debug Draw and Build > Clear Problems: the build's problems and the compiler's
/// errors the viewport draws go.
pub fn clear_debug_draw(editor: &Arc<Editor>) {
    run(editor, "level.problems", args(&[("clear", Value::Bool(true))]));
}

// ---- the command table's rows -----------------------------------------------------------------------

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
    // the tools by their keys: Place with that kind armed (a painted layer's or the cells' mode)
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
            CppAction::shell(|cx, p1, _| {
                let svc = need_scene(cx)?;
                let class = p1.int().and_then(ObjClass::from_id).ok_or("no such class")?;
                svc.store.post(SceneCmd::Arm(class), cx.editor.ui_origin());
                ok()
            }),
        )
        .subs(subs)
        .face("Tools", "Tool", Icon::None)
        .enabled(has_scene),
    );
    // a kind's eye and lock (`p1` the kind, `p2` on)
    for (id, name, lock) in [(59, "COMMAND_READONLY_TARGET", true), (60, "COMMAND_SHOW_TARGET", false)] {
        b.cpp_command(
            CppCommand::new(
                id,
                name,
                "",
                CppAction::shell(move |cx, p1, p2| {
                    let svc = need_scene(cx)?;
                    let class = p1.int().and_then(ObjClass::from_id).ok_or("no such class")?;
                    let on = p2.truthy();
                    let (visible, locked) = if lock { (None, Some(on)) } else { (Some(on), None) };
                    svc.store.post(SceneCmd::ClassView { class, visible, locked }, cx.editor.ui_origin());
                    ok()
                }),
            )
            .editable(false)
            .enabled(has_scene),
        );
    }
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
                    // a person gets the Open Scene dialog; a caller who cannot answer one is told
                    // what to pass instead
                    None => open_scene_dialog(cx.editor, cx.ctx).map_err(|_| NAME_THE_SCENE.to_owned())?,
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
                // the Save Scene As dialog for a person, the argument to pass for any other caller
                let ask_file =
                    |cx: &mut DocCx<'_>| save_as(cx.editor, cx.ctx).map_err(|_| NAME_THE_SAVE.to_owned());
                if p2.int() == Some(1) {
                    ask_file(cx)?;
                    return ok();
                }
                let file = p1.text().filter(|f| !f.is_empty()).map(str::to_owned);
                match file {
                    Some(f) => run(cx.editor, "scene.save", args(&[("path", json!(f))])),
                    None => match scene(cx.editor).and_then(|s| s.store.snapshot().meta.file.clone()) {
                        Some(_) => run(cx.editor, "scene.save", Args::new()),
                        None => ask_file(cx)?,
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
        .enabled(|ed, _| ed.project_opt().is_some() && !recent(ed).is_empty()),
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
                    Some(f) => {
                        run(cx.editor, "scene.place", args(&[("kind", json!("group")), ("ref", json!(f))]))
                    }
                    None => place_group(cx.editor, cx.ctx)?,
                }
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Place Group...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            78,
            "COMMAND_SAVE_SELECTION",
            "",
            CppAction::shell(|cx, p1, _| {
                match p1.text() {
                    Some(f) => {
                        run(cx.editor, "scene.group", args(&[("action", json!("save")), ("path", json!(f))]))
                    }
                    None => save_group(cx.editor, cx.ctx)?,
                }
                ok()
            }),
        )
        .editable(false)
        .face("Scene", "Save Selection as Group...", Icon::None)
        .enabled(has_scene),
    );
    b.cpp_command(
        CppCommand::new(
            69,
            "COMMAND_CLEAR_DEBUG_DRAW",
            "",
            CppAction::editor("level.problems", |_, _| Ok(args(&[("clear", json!(true))]))),
        )
        .editable(false)
        .face("Scene", "Clear Debug Draw", Icon::None)
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
    b.cpp_command(
        CppCommand::new(
            82,
            "COMMAND_COLLECT_SCENE_SUMMARY",
            "",
            CppAction::editor("scene.info", |_, _| Ok(args(&[("summary", json!(true))]))),
        )
        .editable(false)
        .enabled(has_scene),
    );
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
            CppAction::editor("scene.info", |p1, _| {
                let f = p1.text().ok_or("p1s names the file")?;
                Ok(args(&[("summary", json!(true)), ("path", json!(f))]))
            }),
        )
        .editable(false)
        .enabled(has_scene),
    );
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
        id: SmolStr::new("reset_surfaces_question"),
        window: SmolStr::new("Reset Surfaces###dlg.reset_surfaces"),
        needs_project: true,
        open: Box::new(|cx| {
            crate::tools::ask_reset_all(cx.editor, cx.ctx, gz_ui::Asker::Ui);
            Ok(())
        }),
        draw: Box::new(|_, _| {}),
    });
    // the Group tool's Align to Object...: the chooser of the selected group's objects, to be
    // looked at; `scene.group action=pivot_to object=` is the pick
    b.dialog(DialogDesc {
        id: SmolStr::new("group_pivot"),
        window: SmolStr::new("Select Object###dlg.choose"),
        needs_project: true,
        open: Box::new(|cx| {
            let svc = scene(cx.editor).ok_or_else(|| "no scene is open".to_owned())?;
            if crate::tools::pivot_chooser(&svc.store.snapshot()).is_none() {
                return Err("Select a group first".to_owned());
            }
            svc.view.lock().tool_chooser = Some("group_pivot".to_owned());
            cx.editor.exec.wake().ping();
            Ok(())
        }),
        draw: Box::new(|_, _| {}),
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
            let _ = open_scene_dialog(&editor, cx.ctx);
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
            let _ = save_as(&editor, cx.ctx);
        }
    }
}
