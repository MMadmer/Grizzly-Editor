//! What the Level tab's actions need of a person: the system's file dialogs (Open Scene...,
//! Save As..., Place Group..., Save Selection as Group..., Export as OBJ..., Compile a Level
//! Folder...), the question before unsaved scene work is lost, the Summary Info window, Delete in
//! the painting modes, and the File menu's Recent Scene list.
//!
//! Each ends in an editor command (`scene.open`, `scene.save`, `scene.group`, `scene.info`,
//! `level.export`, `level.compile` ...): a caller who cannot answer a dialog - an agent, `-nodlg` -
//! names the file in that command instead, and `level.show` tells it so. File dialogs open on a
//! thread of their own.

use std::path::PathBuf;
use std::sync::Arc;

use egui::Context;
use gz_app::{DialogDesc, DocCx, MenuContent, MenuEntry, ShellBuilder};
use gz_editor::{Args, Editor, SmolStr, Value};
use gz_scene::{SceneCmd, SceneMode, SceneService};
use gz_ui::file_dialog::FileRequest;
use gz_ui::{Answer, Dialog, DialogSize, Icon, MenuItem, menu_item};
use parking_lot::Mutex;

use crate::actions::{recent, scene};
use crate::tools::asker;

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
/// caller who cannot answer it nothing opens and the answer says to name the file.
fn file_dialog(
    editor: &Arc<Editor>,
    ctx: &Context,
    request: FileRequest,
    then: impl FnOnce(&Arc<Editor>, PathBuf) + Send + 'static,
) -> Result<(), String> {
    let ed = editor.clone();
    gz_ui::file_dialog::pick(Some(ctx), asker(editor), request, move |f| then(&ed, f)).map_err(|line| {
        editor.log.finding(&line);
        gz_ui::file_dialog::NAME_THE_FILE.to_owned()
    })
}

fn open_request(title: &str, filter: (&str, &[&str]), dir: PathBuf) -> FileRequest {
    FileRequest::open(title).filter(filter.0, filter.1).in_folder(dir)
}

fn save_request(title: &str, filter: (&str, &[&str]), dir: PathBuf, name: &str) -> FileRequest {
    FileRequest::save(title, name).filter(filter.0, filter.1).in_folder(dir).creating_folder()
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
        editor.log.finding(&l);
    }
}

/// Opens a scene file, asking about unsaved work first.
pub fn open_scene_file(editor: &Arc<Editor>, ctx: &Context, file: String) {
    settle_then(editor, ctx, move |ed, mut a| {
        a.insert("path".into(), Value::String(file));
        run(ed, "scene.open", a);
        ed.post("ui.tab", args(&[("action", Value::from("activate")), ("doc", Value::from(crate::LEVEL))]));
    });
}

/// What a window of the Level tab answers a caller no dialog opens for.
pub const NAME_IT: &str = "no file dialog opens for this caller: name the file in the command itself";

/// The windows `level.show dialog=` opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LevelDialog {
    /// Open Scene...: a `.scene` of the project, or an X-Ray SDK `.level` to import (`scene.open path=`).
    OpenScene,
    /// Save As... (`scene.save path=`).
    SaveAs,
    /// Place Group...: a `.group` placed in front of the camera (`scene.place kind=group ref=`).
    PlaceGroup,
    /// Save Selection as Group... (`scene.group action=save path=`).
    SaveGroup,
    /// Export Scene as OBJ... (`level.export kind=obj all=true path=`).
    ExportObj,
    /// Export Selection as OBJ... (`level.export kind=obj path=`).
    ExportSelectionObj,
    /// Compile a Level Folder... (`level.compile folder=`).
    CompileFolder,
    /// The Summary Info window (`scene.info summary=true`).
    Summary,
}

impl LevelDialog {
    /// The command a caller who cannot answer a file dialog runs instead.
    #[must_use]
    pub fn instead(self) -> &'static str {
        match self {
            LevelDialog::OpenScene => "scene.open path=<file>",
            LevelDialog::SaveAs => "scene.save path=<file>",
            LevelDialog::PlaceGroup => "scene.place kind=group ref=<file>",
            LevelDialog::SaveGroup => "scene.group action=save path=<file>",
            LevelDialog::ExportObj => "level.export kind=obj all=true path=<file>",
            LevelDialog::ExportSelectionObj => "level.export kind=obj path=<file>",
            LevelDialog::CompileFolder => "level.compile folder=<folder>",
            LevelDialog::Summary => "scene.info summary=true",
        }
    }
}

/// Opens one of the Level tab's windows for a person.
pub fn open(editor: &Arc<Editor>, ctx: &Context, which: LevelDialog) -> Result<(), String> {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else {
        return Err("Open a project first.".to_owned());
    };
    let root = svc.root.clone();
    match which {
        LevelDialog::OpenScene => {
            let cx = ctx.clone();
            file_dialog(
                editor,
                ctx,
                open_request("Open Scene", ("Scene", &["scene", "level"]), root),
                move |ed, f| open_scene_file(ed, &cx, f.display().to_string()),
            )
        }
        LevelDialog::SaveAs => save_as(editor, ctx),
        LevelDialog::PlaceGroup => {
            file_dialog(editor, ctx, open_request("Place Group", ("Group", &["group"]), root), |ed, f| {
                run(ed, "scene.place", args(&[("kind", Value::from("group")), ("ref", path_text(&f))]));
            })
        }
        LevelDialog::SaveGroup => file_dialog(
            editor,
            ctx,
            save_request(
                "Save Selection as Group",
                ("Group", &["group"]),
                root.join("groups"),
                "selection.group",
            ),
            |ed, f| run(ed, "scene.group", args(&[("action", Value::from("save")), ("path", path_text(&f))])),
        ),
        LevelDialog::ExportObj | LevelDialog::ExportSelectionObj => {
            let all = which == LevelDialog::ExportObj;
            file_dialog(
                editor,
                ctx,
                save_request("Export as OBJ", ("Wavefront OBJ", &["obj"]), root.join("import"), "scene.obj"),
                move |ed, f| {
                    run(
                        ed,
                        "level.export",
                        args(&[
                            ("kind", Value::from("obj")),
                            ("path", path_text(&f)),
                            ("all", Value::Bool(all)),
                        ]),
                    );
                },
            )
        }
        LevelDialog::CompileFolder => file_dialog(
            editor,
            ctx,
            FileRequest::folder("Compile a Level Folder").in_folder(root.join("gamedata").join("levels")),
            |ed, f| run(ed, "level.compile", args(&[("folder", path_text(&f))])),
        ),
        LevelDialog::Summary => {
            summary(editor);
            Ok(())
        }
    }
}

/// File > Save As...: the system's dialog in the project.
pub fn save_as(editor: &Arc<Editor>, ctx: &Context) -> Result<(), String> {
    let Some(svc) = scene(editor) else { return Err("Open or create a scene first.".to_owned()) };
    let snap = svc.store.snapshot();
    let name = snap
        .meta
        .file
        .as_ref()
        .and_then(|f| f.file_name())
        .map_or_else(|| "level.scene".to_owned(), |n| n.to_string_lossy().into_owned());
    file_dialog(
        editor,
        ctx,
        save_request("Save Scene As", ("Scene", &["scene"]), svc.root.clone(), &name),
        |ed, f| {
            run(ed, "scene.save", args(&[("path", path_text(&f))]));
        },
    )
}

// ---- Edit ---------------------------------------------------------------------------------------

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

/// Edit > Delete: the selection of whatever kinds it holds, as one undo step; in the AI map's
/// and the decals' painting modes the nodes or the marks selected there.
pub fn delete(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let snap = svc.store.snapshot();
    match snap.tools.mode {
        SceneMode::AiMap => run(editor, "scene.ai_map", args(&[("action", Value::from("remove"))])),
        SceneMode::Wallmarks => run(editor, "scene.wallmarks", args(&[("action", Value::from("remove"))])),
        // the foliage has no selection of its own
        SceneMode::Details => {}
        _ => svc.store.post(SceneCmd::RemoveSelection { label: "Delete".into() }, editor.ui_origin()),
    }
}

// ---- Summary Info -------------------------------------------------------------------------------

/// The Summary Info window's rows, shown in `dlg.scene_summary`.
#[derive(Default)]
struct SummarySheet {
    open: bool,
    rows: Vec<(String, String)>,
    busy: bool,
}

fn sheet(editor: &Editor) -> Arc<Mutex<SummarySheet>> {
    editor.services.get_or_init(|| Mutex::new(SummarySheet::default()))
}

/// Scene > Summary Info: what the scene holds and uses, counted off the UI thread.
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

/// Whether the Summary Info window is up.
#[must_use]
pub fn summary_open(editor: &Editor) -> bool {
    sheet(editor).lock().open
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
                    "Counting",
                    "What the scene holds and uses is counted.",
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
                save_request(
                    "Export Summary",
                    ("Text", &["txt"]),
                    root.join("import"),
                    &format!("{level}.txt"),
                ),
                |ed, f| {
                    run(ed, "scene.info", args(&[("summary", Value::Bool(true)), ("path", path_text(&f))]))
                },
            );
        }
    }
    s.lock().open = open && !close;
}

// ---- the shell's rows ---------------------------------------------------------------------------

/// The File menu's Recent Scene list and the Level tab's dialogs by id.
pub(crate) fn install(b: &mut ShellBuilder) {
    b.menu_entry(MenuEntry {
        menu: SmolStr::new("File"),
        submenu: None,
        group: 10,
        order: 2,
        content: MenuContent::Custom(Box::new(recent_rows)),
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

/// File > Recent Scene: the project's scenes opened last on this machine.
fn recent_rows(ui: &mut egui::Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    let list = recent(&editor);
    gz_app::chrome::local_submenu(
        ui,
        "Recent Scene",
        Icon::None,
        editor.project_opt().is_some() && !list.is_empty(),
        |ui| {
            for f in &list {
                if menu_item(ui, f, MenuItem::default()) {
                    open_scene_file(&editor, cx.ctx, f.clone());
                }
            }
        },
    );
}
