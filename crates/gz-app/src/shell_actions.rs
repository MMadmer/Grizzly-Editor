//! The window's own actions: File, Edit, Window and Help, and the keys the frame keeps.

use egui::Key;
use gz_editor::{Args, Value, json};
use gz_ui::Icon;

use crate::actions::{Action, ActionCx, Enabled, args};
use crate::doc::{Drawer, EditAction};
use crate::keys::Chord;

fn active_doc(cx: &ActionCx<'_>) -> Args {
    let mut a = Args::new();
    a.insert("doc".into(), json!(cx.tab));
    a
}

fn edit_args(cx: &ActionCx<'_>, what: &str) -> Args {
    let mut a = active_doc(cx);
    a.insert("action".into(), json!(what));
    a
}

fn on_a_document(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(cx.project, "Open a project first.")
}

fn edit_on(cx: &ActionCx<'_>, a: EditAction) -> Enabled {
    Enabled::when(cx.project && cx.takes(a), "This tab has nothing to do with it.")
}

fn running_job(cx: &ActionCx<'_>) -> Option<u64> {
    cx.editor.jobs.snapshot().into_iter().rev().find(gz_editor::JobInfo::running).map(|j| j.id.0)
}

/// The active tab's file shown in the Content Browser, and in Explorer.
pub const BROWSE_TAB: &str = "edit.show_in_browser";
pub const EXPLORE_TAB: &str = "edit.show_in_explorer";

/// `content.view` revealing a project file (project-relative).
#[must_use]
pub fn reveal_in_browser(path: &str) -> Args {
    args(&[("action", "reveal"), ("source", "project"), ("path", path)])
}

/// Every action of the window itself.
#[must_use]
pub fn shell_actions() -> Vec<Action> {
    vec![
        // File
        Action::new("file.save", "Save", "File", "doc.save")
            .icon(Icon::Save)
            .keys(const { &[Chord::ctrl(Key::S)] })
            .args(active_doc)
            .enabled(|cx| Enabled::when(cx.project && cx.dirty, "Nothing on this tab to save."))
            .frame_key(),
        Action::new("file.save_all", "Save All", "File", "doc.save_all")
            .icon(Icon::Save)
            .keys(const { &[Chord::ctrl_shift(Key::S)] })
            .enabled(on_a_document)
            .frame_key(),
        Action::new("project.open_folder", "Open Project Folder", "File", "ui.reveal")
            .icon(Icon::Reveal)
            .enabled(on_a_document),
        Action::new("project.switch", "Switch Project...", "File", "app.switch_project")
            .enabled(on_a_document),
        Action::new("app.quit", "Quit", "File", "app.quit").icon(Icon::Close),
        // Edit: what the active tab does with them
        Action::new("edit.undo", "Undo", "Edit", "doc.undo")
            .icon(Icon::Undo)
            .keys(const { &[Chord::ctrl(Key::Z)] })
            .args(active_doc)
            .enabled(|cx| edit_on(cx, EditAction::Undo)),
        Action::new("edit.redo", "Redo", "Edit", "doc.redo")
            .icon(Icon::Redo)
            .keys(const { &[Chord::ctrl(Key::Y), Chord::ctrl_shift(Key::Z)] })
            .args(active_doc)
            .enabled(|cx| edit_on(cx, EditAction::Redo)),
        Action::new("edit.cut", "Cut", "Edit", "doc.edit")
            .icon(Icon::Cut)
            .keys(const { &[Chord::ctrl(Key::X)] })
            .args(|cx| edit_args(cx, "cut"))
            .enabled(|cx| edit_on(cx, EditAction::Cut)),
        Action::new("edit.copy", "Copy", "Edit", "doc.edit")
            .icon(Icon::Copy)
            .keys(const { &[Chord::ctrl(Key::C)] })
            .args(|cx| edit_args(cx, "copy"))
            .enabled(|cx| edit_on(cx, EditAction::Copy)),
        Action::new("edit.paste", "Paste", "Edit", "doc.edit")
            .icon(Icon::Paste)
            .keys(const { &[Chord::ctrl(Key::V)] })
            .args(|cx| edit_args(cx, "paste"))
            .enabled(|cx| edit_on(cx, EditAction::Paste)),
        Action::new("edit.duplicate", "Duplicate", "Edit", "doc.edit")
            .icon(Icon::Duplicate)
            .keys(const { &[Chord::ctrl(Key::D)] })
            .args(|cx| edit_args(cx, "duplicate"))
            .enabled(|cx| edit_on(cx, EditAction::Duplicate)),
        Action::new("edit.delete", "Delete", "Edit", "doc.edit")
            .icon(Icon::Delete)
            .keys(const { &[Chord::new(Key::Delete)] })
            .args(|cx| edit_args(cx, "delete"))
            .enabled(|cx| edit_on(cx, EditAction::Delete)),
        Action::new("edit.find", "Find", "Edit", "doc.edit")
            .icon(Icon::Search)
            .keys(const { &[Chord::ctrl(Key::F)] })
            .args(|cx| edit_args(cx, "find"))
            .enabled(|cx| edit_on(cx, EditAction::Find)),
        Action::new("edit.palette", "Command Palette...", "Edit", "ui.palette")
            .icon(Icon::Search)
            .keys(const { &[Chord::ctrl_shift(Key::P)] })
            .args(|_| args(&[("action", "open")]))
            .hidden_from_palette()
            .frame_key(),
        Action::new(BROWSE_TAB, "Show in Content Browser", "Edit", "content.view")
            .icon(Icon::Folder)
            .keys(const { &[Chord::ctrl(Key::B)] })
            .args(|cx| reveal_in_browser(cx.path))
            .enabled(|cx| {
                if !cx.project || cx.path.is_empty() {
                    return Enabled::Off("This tab shows no file of the project.");
                }
                // what the browser shows is the project: a file outside it is Explorer's alone
                Enabled::when(
                    !std::path::Path::new(cx.path).is_absolute(),
                    "The file is outside the project.",
                )
            }),
        Action::new(EXPLORE_TAB, "Show in Explorer", "Edit", "ui.reveal")
            .icon(Icon::Reveal)
            .args(|cx| args(&[("path", cx.path)]))
            .enabled(|cx| {
                Enabled::when(cx.project && !cx.path.is_empty(), "This tab shows no file of the project.")
            }),
        Action::new("app.preferences", "Preferences", "Edit", "app.prefs")
            .icon(Icon::Settings)
            .args(|_| args(&[("action", "show")])),
        Action::new("job.stop_running", "Stop the Running Job", "Edit", "job.stop")
            .icon(Icon::Stop)
            .args(|cx| {
                let mut a = Args::new();
                if let Some(id) = running_job(cx) {
                    a.insert("job".into(), json!(id));
                }
                a
            })
            .enabled(|cx| Enabled::when(running_job(cx).is_some(), "No job runs now.")),
        // Window
        Action::new("window.content_drawer", "Content Drawer", "Window", "ui.drawer")
            .icon(Icon::Drawer)
            .keys(const { &[Chord::ctrl(Key::Space)] })
            .args(|_| args(&[("action", "toggle"), ("id", "content")]))
            .enabled(on_a_document)
            .checked(|cx| cx.drawer == Some(Drawer::Content))
            .frame_key(),
        Action::new("window.log_drawer", "Log Drawer", "Window", "ui.drawer")
            .icon(Icon::Log)
            .keys(const { &[Chord::new(Key::Backtick)] })
            .args(|_| args(&[("action", "toggle"), ("id", "log")]))
            .enabled(on_a_document)
            .checked(|cx| cx.drawer == Some(Drawer::Log))
            .frame_key(),
        Action::new("window.next_tab", "Next Tab", "Window", "ui.tab")
            .keys(const { &[Chord::ctrl(Key::Tab)] })
            .args(|_| args(&[("action", "next")]))
            .enabled(on_a_document)
            .frame_key(),
        Action::new("window.previous_tab", "Previous Tab", "Window", "ui.tab")
            .keys(const { &[Chord::ctrl_shift(Key::Tab)] })
            .args(|_| args(&[("action", "prev")]))
            .enabled(on_a_document)
            .frame_key(),
        Action::new("window.close_tab", "Close Tab", "Window", "ui.tab")
            .icon(Icon::Close)
            .keys(const { &[Chord::ctrl(Key::F4)] })
            .args(|cx| {
                let mut a = args(&[("action", "close")]);
                a.insert("doc".into(), Value::String(cx.tab.to_owned()));
                a
            })
            .enabled(|cx| {
                Enabled::when(cx.project && cx.tab != crate::tabs::LEVEL, "The Level tab stays open.")
            })
            .frame_key(),
        Action::new("window.reset_layout", "Reset Layout", "Window", "ui.reset_layout")
            .icon(Icon::Refresh)
            .enabled(on_a_document),
        // Help
        Action::new("help.keyboard", "Keyboard Shortcuts", "Help", "app.prefs")
            .icon(Icon::Keyboard)
            .args(|_| args(&[("action", "show"), ("page", "Keyboard")])),
        Action::new("help.about", "About Grizzly Editor", "Help", "app.about").icon(Icon::Info),
    ]
}

/// Where the window's own actions stand in its menus: menu, group, order, action, label.
pub const SHELL_MENU: &[(&str, i32, i32, &str, Option<&str>)] = &[
    ("File", 20, 0, "file.save", None),
    ("File", 20, 1, "file.save_all", None),
    ("File", 90, 0, "app.quit", None),
    ("Edit", 0, 0, "edit.undo", None),
    ("Edit", 0, 1, "edit.redo", None),
    ("Edit", 10, 0, "edit.cut", None),
    ("Edit", 10, 1, "edit.copy", None),
    ("Edit", 10, 2, "edit.paste", None),
    ("Edit", 10, 3, "edit.duplicate", None),
    ("Edit", 10, 4, "edit.delete", None),
    ("Edit", 20, 0, "edit.find", None),
    ("Edit", 30, 0, BROWSE_TAB, None),
    ("Edit", 30, 1, EXPLORE_TAB, None),
    ("Edit", 90, 0, "edit.palette", None),
    ("Edit", 90, 1, "app.preferences", None),
    ("Window", 50, 0, "window.content_drawer", None),
    ("Window", 50, 1, "window.log_drawer", None),
    ("Window", 90, 0, "window.reset_layout", None),
    ("Help", 0, 0, "help.keyboard", None),
    ("Help", 0, 1, "help.about", None),
];

/// The rows of File > Project.
pub const PROJECT_MENU: &[(i32, i32, &str)] = &[(50, 0, "project.open_folder"), (60, 0, "project.switch")];
