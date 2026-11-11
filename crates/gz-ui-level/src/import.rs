//! Import Base Scene (`Import Base Scene###dlg.import_base_scene`, M; `EditorProject::DrawUI`):
//! the editor library's source scenes with a search, "In project" on what the project holds;
//! Import copies one into the project (`gz_project::project::import_base_scene`, on the IO
//! pool). The list and what the project holds of it are read once, as the dialog opens.

use std::sync::Arc;

use egui::{Align2, Rect, Ui, Vec2, pos2};
use gz_app::{DialogDesc, DocCx, ShellBuilder};
use gz_editor::{Editor, SmolStr};
use gz_ui::text::{self, Clip};
use gz_ui::{Dialog, DialogSize, Font, Icon, Kind, SearchField, Tone, icons, list, palette, state};
use parking_lot::Mutex;

/// What the dialog read as it opened.
#[derive(Clone, Debug, Default)]
struct Listed {
    scenes: Vec<String>,
    in_project: Vec<bool>,
    library: String,
}

#[derive(Default)]
struct ImportState {
    open: bool,
    listed: Option<Listed>,
    reading: bool,
    search: String,
    pick: String,
}

/// The dialog's state: an `Editor` service (the dialog lives as long as the window).
#[derive(Default)]
struct ImportDialog(Mutex<ImportState>);

fn state_of(editor: &Editor) -> Arc<ImportDialog> {
    editor.services.get_or_init(ImportDialog::default)
}

fn open(cx: &mut DocCx<'_>) -> Result<(), String> {
    let editor = cx.editor.clone();
    let st = state_of(&editor);
    let mut s = st.0.lock();
    s.open = true;
    s.listed = None;
    s.reading = true;
    s.search.clear();
    s.pick.clear();
    drop(s);
    let root = editor.paths.root.clone();
    let project = editor.project_root();
    let ed = editor.clone();
    editor.exec.spawn_io(move || {
        // with no project open there is nothing to import into: the list stays empty
        let scenes = if project.is_some() { gz_project::project::base_scenes(&root) } else { Vec::new() };
        let in_project = scenes
            .iter()
            .map(|n| {
                project
                    .as_ref()
                    .is_some_and(|p| p.join("rawdata").join("levels").join(format!("{n}.level")).exists())
            })
            .collect();
        let library = root.join("rawdata").join("levels").display().to_string();
        let st = state_of(&ed);
        let mut s = st.0.lock();
        s.listed = Some(Listed { scenes, in_project, library });
        s.reading = false;
        drop(s);
        ed.exec.wake().ping();
    });
    Ok(())
}

fn import(editor: &Arc<Editor>, name: String, there: bool) {
    let Some(project) = editor.project_root() else { return };
    let root = editor.paths.root.clone();
    let ed = editor.clone();
    editor.exec.spawn_io(move || {
        let result = gz_project::project::import_base_scene(&root, &project, &name);
        let Ok(queue) = gz_app::UiQueue::of(&ed) else { return };
        queue.post(move |shell, ctx| match result {
            Ok(_) => {
                let text_ = if there {
                    format!("Scene '{name}' is in the project already.")
                } else {
                    format!("Scene '{name}' imported into the project.")
                };
                state::toast(ctx, Tone::Success, &text_);
                state_of(&ed).0.lock().open = false;
            }
            Err(e) => {
                let asker = if ed.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
                if let Some(line) = gz_ui::dialog::notice(ctx, asker, Tone::Error, "Import Base Scene", &e) {
                    ed.log.plain(line);
                }
                let _ = shell;
            }
        });
    });
}

fn draw(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    let st = state_of(&editor);
    let mut s = st.0.lock();
    if !s.open {
        return;
    }
    let p = palette(ui.ctx());
    let mut open = true;
    let mut do_import = false;
    let mut import_pressed = false;
    let mut cancel = false;
    let listed = s.listed.clone();
    let count = listed.as_ref().map_or(0, |l| l.scenes.len());
    let status = format!("{count} {}", if count == 1 { "scene" } else { "scenes" });
    let can_import = !s.pick.is_empty();
    let ImportState { search, pick, reading, .. } = &mut *s;
    Dialog::new("Import Base Scene", "dlg.import_base_scene", DialogSize::M).show(
        ui,
        &mut open,
        |ui| {
            SearchField::new(search).hint("Search scenes").id_salt("import_search").show(ui);
            ui.add_space(4.0);
            let Some(l) = &listed else {
                if *reading {
                    state::busy_line(ui, "Reading the library...");
                }
                return;
            };
            let badge_w = text::width(ui.ctx(), &Font::Small.id(), "In project") + 12.0;
            list::list(ui, "import_scenes", Vec2::new(0.0, 320.0), |lst| {
                let mut shown = 0;
                for (i, scene) in l.scenes.iter().enumerate() {
                    if !gz_ui::search::matches(search, scene) {
                        continue;
                    }
                    shown += 1;
                    let there = l.in_project.get(i).copied().unwrap_or(false);
                    let resp = lst.row(scene, *pick == *scene, |ui| {
                        let r = ui.max_rect();
                        icons::draw(ui.painter(), Icon::Level, Rect::from_min_size(pos2(r.min.x, r.center().y - 7.0), Vec2::splat(14.0)), p.text_secondary);
                        let x = r.min.x + 20.0;
                        let room = (r.max.x - x - 6.0 - if there { badge_w + 8.0 } else { 0.0 }).max(0.0);
                        text::paint(ui.painter(), &Font::Body.id(), pos2(x, r.center().y), Align2::LEFT_CENTER, p.text, scene, Some(room), Clip::End);
                        if there {
                            let br = Rect::from_min_size(pos2(r.max.x - 6.0 - badge_w, r.center().y - 9.0), Vec2::new(badge_w, 18.0));
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
                            state::badge(&mut child, "In project", Tone::Muted);
                        }
                    });
                    if resp.clicked() {
                        pick.clone_from(scene);
                    }
                    if resp.double_clicked() {
                        pick.clone_from(scene);
                        do_import = true;
                    }
                }
                if shown == 0 {
                    text::label(lst.ui(), if l.scenes.is_empty() { "The editor library holds no scenes." } else { "No scene matches the search." }, Font::Body, p.text_secondary, Clip::End);
                }
            });
            ui.add_space(4.0);
            text::label(ui, &l.library, Font::Body, p.text_muted, Clip::Start)
                .on_hover_text("The editor's library of the game's source scenes. Importing copies one into the project; the library stays as it is.");
        },
        |f| {
            f.status(&status, Tone::Muted);
            import_pressed = f.button("Import", Kind::Primary, true, can_import, "Pick a scene first.");
            if f.button("Cancel", Kind::Secondary, false, true, "") {
                cancel = true;
            }
        },
    );
    do_import |= import_pressed;
    if do_import && !s.pick.is_empty() {
        let there = s
            .listed
            .as_ref()
            .and_then(|l| {
                l.scenes.iter().position(|n| *n == s.pick).and_then(|i| l.in_project.get(i).copied())
            })
            .unwrap_or(false);
        import(&editor, s.pick.clone(), there);
    }
    if cancel || !open {
        s.open = false;
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.dialog(DialogDesc {
        id: SmolStr::new("import_base_scene"),
        window: SmolStr::new("Import Base Scene###dlg.import_base_scene"),
        needs_project: false,
        open: Box::new(open),
        draw: Box::new(draw),
    });
    b.menu_entry(gz_app::MenuEntry {
        menu: SmolStr::new("File"),
        submenu: Some(SmolStr::new("Project")),
        group: 40,
        order: 0,
        content: gz_app::MenuContent::Custom(Box::new(|ui: &mut Ui, cx: &mut DocCx<'_>| {
            let on = cx.editor.project_opt().is_some();
            if gz_ui::menu_item(
                ui,
                "Import Base Scene...",
                gz_ui::MenuItem { enabled: on, ..gz_ui::MenuItem::default() },
            ) {
                cx.requests.dialog("import_base_scene");
            }
        })),
    });
}
