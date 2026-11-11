//! The motion rows' buttons that need more than a value (`CCustomObject::OnMotionControlClick`,
//! `OnMotionFilesClick`, `OnMotionCommandsClick`): Play and Pause on the viewport's frames, the
//! `.anm` files through the system's dialogs, and the Scale Keys / Normalize Keys sheets
//! (`dlg.scale_keys`, `dlg.normalize_keys`). Each ends in `object_motion`.

use std::sync::Arc;

use egui::Ui;
use gz_editor::{Args, Editor, Value, json};
use gz_scene::SceneService;
use gz_ui::{Dialog, DialogSize, Kind};
use parking_lot::Mutex;

/// The open sheet: Scale (`true`) or Normalize, the object, From Time, To Time, the factor.
#[derive(Clone, Debug)]
struct Sheet {
    scale: bool,
    object: String,
    from: f32,
    to: f32,
    value: f32,
}

#[derive(Default)]
struct MotionUi {
    sheet: Option<Sheet>,
}

fn state(editor: &Editor) -> Arc<Mutex<MotionUi>> {
    editor.services.get_or_init(|| Mutex::new(MotionUi::default()))
}

fn motion_call(editor: &Arc<Editor>, object: &str, mut a: Args) {
    a.insert("name".into(), json!(object));
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let r = ed.call("object_motion", a, ed.ui_origin()).await;
        if r.get("ok").and_then(Value::as_bool) != Some(true) {
            ed.notices.error(r.get("error").and_then(Value::as_str).unwrap_or("The motion command failed.").to_owned());
        }
    });
}

/// The first selected object of the current tool: the one the motion rows are about.
fn selected_object(editor: &Editor) -> Option<(String, Option<(f32, f32)>)> {
    let svc = editor.project_opt()?.services.get::<SceneService>()?;
    let snap = svc.store.snapshot();
    let class = snap.tools.target;
    let o = snap.of_class(class).find(|o| snap.is_selected(o.id))?;
    let range = o.motion.as_ref().and_then(|r| r.model().map(|m| gz_scene::objmotion::params(&m, r))).map(|p| (p.min_t, p.max_t));
    Some((o.name.to_string(), range))
}

/// A press of a motion button the row itself does not do; `true` when it was one.
pub fn press(editor: &Arc<Editor>, key: &str, caption: &str) -> bool {
    let Some((object, range)) = selected_object(editor) else { return false };
    let action = |a: &str| {
        let mut args = Args::new();
        args.insert("action".into(), json!(a));
        args
    };
    match (key, caption) {
        ("Motion\\Controls", "Play") => motion_call(editor, &object, action("play")),
        ("Motion\\Controls", "Pause") => motion_call(editor, &object, action("pause")),
        ("Motion\\Files", "Import" | "Export") => {
            let import = caption == "Import";
            let Some(root) = editor.project_root() else { return true };
            let dir = root.join("gamedata").join("anims");
            let ed = editor.clone();
            let spawned = std::thread::Builder::new().name("gz-ui-level.motion_file".into()).spawn(move || {
                let _ = std::fs::create_dir_all(&dir);
                let d = rfd::FileDialog::new().add_filter("Object motion", &["anm"]).set_directory(&dir);
                let picked = if import {
                    d.set_title("Import Motion").pick_file()
                } else {
                    d.set_title("Export Motion").set_file_name(format!("{object}.anm")).save_file()
                };
                if let Some(f) = picked {
                    let mut a = Args::new();
                    a.insert("action".into(), json!(if import { "import" } else { "export" }));
                    a.insert("file".into(), json!(f.display().to_string()));
                    motion_call(&ed, &object, a);
                }
            });
            if let Err(e) = spawned {
                editor.log.warn(format!("the file dialog did not open: {e}"));
            }
        }
        ("Motion\\Commands", "Scale Keys..." | "Normalize Keys...") => {
            let (from, to) = range.unwrap_or((0.0, 0.0));
            let scale = caption == "Scale Keys...";
            state(editor).lock().sheet =
                Some(Sheet { scale, object, from, to, value: if scale { 1.0 } else { 5.0 } });
        }
        _ => return false,
    }
    true
}

/// Draws the open sheet; OK sends it.
pub fn draw(ui: &mut Ui, editor: &Arc<Editor>) {
    let st = state(editor);
    let Some(mut sheet) = st.lock().sheet.clone() else { return };
    let mut open = true;
    let (mut ok, mut cancel) = (false, false);
    let (title, name) =
        if sheet.scale { ("Scale Keys", "dlg.scale_keys") } else { ("Normalize Keys", "dlg.normalize_keys") };
    Dialog::new(title, name, DialogSize::S).show(
        ui,
        &mut open,
        |ui| {
            gz_ui::PropertyGrid::new("motion_keys").show(ui, |g| {
                g.row("From Time", "The first key the change takes, in seconds", gz_ui::PropFlags::NONE, 0, |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.from).speed(1.0 / 30.0).max_decimals(3));
                });
                g.row("To Time", "The last key the change takes, in seconds", gz_ui::PropFlags::NONE, 0, |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.to).speed(1.0 / 30.0).max_decimals(3));
                });
                let (label, help, lo, hi) = if sheet.scale {
                    ("Scale", "How much longer the time between the keys becomes", -1000.0, 1000.0)
                } else {
                    ("Speed (m/sec)", "How fast the object moves between the keys", 0.0, 100.0)
                };
                g.row(label, help, gz_ui::PropFlags::NONE, 0, |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.value).speed(0.01).range(lo..=hi).max_decimals(3));
                });
            });
        },
        |f| {
            if f.button("OK", Kind::Primary, true, true, "") {
                ok = true;
            }
            if f.button("Cancel", Kind::Secondary, false, true, "") {
                cancel = true;
            }
        },
    );
    if ok {
        let mut a = Args::new();
        a.insert("action".into(), json!(if sheet.scale { "scale_keys" } else { "normalize_keys" }));
        a.insert("from".into(), json!(sheet.from));
        a.insert("to".into(), json!(sheet.to));
        a.insert(if sheet.scale { "scale" } else { "speed" }.into(), json!(sheet.value));
        motion_call(editor, &sheet.object, a);
    }
    st.lock().sheet = if ok || cancel || !open { None } else { Some(sheet) };
}
