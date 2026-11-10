//! The Level tab's menus - Scene, Build, View (`UIMainMenuForm::DrawLevelMenus`) - and the
//! scene's commands as [`UiCommand`]s: the palette lists them, the Keyboard page binds them and
//! their keys work while the Level tab owns the keys.
//!
//! A menu row shows the keys bound to its command now. A row whose command another crate
//! registers (the level builds, the viewport's view modes) is there only while it is.

use std::sync::Arc;

use egui::{Context, Ui};
use gz_app::chrome::{MenuBar, local_submenu};
use gz_app::{DocCx, EditorSettings, ShellBuilder, UiCommand};
use gz_editor::{Args, Editor, Value};
use gz_scene::apply::SelectMode;
use gz_scene::{SceneCmd, SceneService};
use gz_ui::{Answer, ConfirmSpec, Icon, MenuItem, menu_item};

use crate::doc::{Act, post_to_level};
use crate::toolbar;
use crate::tools::asker;

/// The scene's commands: id, group, label, icon, default keys.
pub const SCENE_COMMANDS: [(&str, &str, &str, Icon, &str); 11] = [
    ("COMMAND_UNDO", "Edit", "Undo", Icon::Undo, "Ctrl+Z"),
    ("COMMAND_REDO", "Edit", "Redo", Icon::Redo, "Ctrl+Y"),
    ("COMMAND_DELETE_SELECTION", "Edit", "Delete", Icon::Delete, "Delete"),
    ("COMMAND_SHOW_OBJECTLIST", "Scene", "Find in World Outliner", Icon::Search, "Ctrl+F"),
    ("COMMAND_SELECT_ALL", "Selection", "Select All", Icon::Select, "Ctrl+A"),
    ("COMMAND_DESELECT_ALL", "Selection", "Unselect All", Icon::Select, ""),
    ("COMMAND_INVERT_SELECTION_ALL", "Selection", "Invert Selection", Icon::Select, "Ctrl+I"),
    ("COMMAND_HIDE_SEL", "Visibility", "Hide Selected", Icon::EyeOff, ""),
    ("COMMAND_HIDE_UNSEL", "Visibility", "Hide Unselected", Icon::EyeOff, ""),
    ("COMMAND_HIDE_ALL", "Visibility", "Hide All", Icon::EyeOff, ""),
    ("COMMAND_UNHIDE_ALL", "Visibility", "Unhide All", Icon::Eye, ""),
];

/// The Build menu's rows past the toolbar's builds: submenu, label, command, its argument.
const BUILD_ROWS: [(&str, &str, &str, &str); 11] = [
    ("Make", "Make All...", "make_all", ""),
    ("Make", "Make Game...", "make_game", ""),
    ("Make", "Make Details...", "make_details", ""),
    ("Make", "Make HOM...", "make_hom", ""),
    ("Make", "Make SOM...", "make_som", ""),
    ("Make", "Make AI Map...", "make_aimap", ""),
    ("Compile", "Geometry & Light", "compile_geometry", ""),
    ("Compile", "Detail Object Light", "compile_details", ""),
    ("Compile", "AI Map High", "compile_aimap", "high"),
    ("Compile", "AI Map Low", "compile_aimap", "low"),
    ("Compile", "Spawn", "compile_spawn", ""),
];

/// What the keys of a command are now: the author's binding, else its default.
#[must_use]
pub fn keys_now(editor: &Editor, id: &str, default: &str) -> String {
    match EditorSettings::of(editor).shortcuts().into_iter().find(|(k, _)| k.eq_ignore_ascii_case(id)) {
        Some((_, keys)) => keys,
        None => default.to_owned(),
    }
}

fn scene(editor: &Editor) -> Option<Arc<SceneService>> {
    editor.project_opt().and_then(|s| s.services.get::<SceneService>()).filter(|s| s.store.snapshot().valid)
}

/// Runs one of [`SCENE_COMMANDS`].
pub fn run_scene_command(id: &str, editor: &Arc<Editor>, ctx: &Context) {
    if id == "COMMAND_SHOW_OBJECTLIST" {
        post_to_level(editor, Act::FindInOutliner);
        return;
    }
    let Some(svc) = scene(editor) else { return };
    let snap = svc.store.snapshot();
    let origin = editor.ui_origin();
    let class = snap.tools.target;
    match id {
        "COMMAND_UNDO" => editor.post("undo", Args::new()),
        "COMMAND_REDO" => editor.post("redo", Args::new()),
        "COMMAND_DELETE_SELECTION" => ask_delete_selection(ctx, editor, &svc),
        "COMMAND_SELECT_ALL" => svc.store.post(SceneCmd::SelectClass { class: Some(class), select: true }, origin),
        "COMMAND_DESELECT_ALL" => svc.store.post(SceneCmd::SelectClass { class: None, select: false }, origin),
        "COMMAND_INVERT_SELECTION_ALL" => {
            let ids = snap.of_class(class).filter(|o| o.flags.visible).map(|o| o.id).collect();
            svc.store.post(SceneCmd::Select { ids, mode: SelectMode::Toggle }, origin);
        }
        "COMMAND_HIDE_SEL" => {
            let ids = snap.selected_in_order();
            if !ids.is_empty() {
                svc.store.post(SceneCmd::Show { ids, visible: false }, origin);
            }
        }
        "COMMAND_HIDE_UNSEL" => {
            let ids: Vec<_> = snap.objects.values().filter(|o| o.class.is_object_tool() && o.flags.visible && !snap.is_selected(o.id)).map(|o| o.id).collect();
            if !ids.is_empty() {
                svc.store.post(SceneCmd::Show { ids, visible: false }, origin);
            }
        }
        "COMMAND_HIDE_ALL" => {
            crate::outliner::show_objects(&svc, false, false, origin);
        }
        "COMMAND_UNHIDE_ALL" => {
            crate::outliner::show_objects(&svc, false, true, origin);
        }
        _ => {}
    }
}

/// Delete asks first, naming what goes (Edit > Undo brings it back).
fn ask_delete_selection(ctx: &Context, editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let snap = svc.store.snapshot();
    let ids: Vec<_> = snap.selected_in_order().into_iter().filter(|id| snap.get(*id).is_some_and(|o| o.owner.is_none())).collect();
    if ids.is_empty() {
        return;
    }
    let names: Vec<String> = ids.iter().filter_map(|id| snap.get(*id)).take(10).map(|o| o.name.to_string()).collect();
    let one = ids.len() == 1;
    let spec = ConfirmSpec {
        id: "delete_object".into(),
        title: if one { "Delete Object" } else { "Delete Objects" }.into(),
        headline: if one { "Delete this object from the scene?".into() } else { format!("Delete these {} objects from the scene?", ids.len()) },
        items: names,
        total: ids.len(),
        consequence: "Edit > Undo brings them back.".into(),
        action: "Delete".into(),
        destructive: true,
    };
    let store = svc.store.clone();
    let origin = editor.ui_origin();
    let line = gz_ui::dialog::confirm(ctx, asker(editor), spec, move |answer| {
        if answer == Answer::Yes {
            store.post(SceneCmd::DeleteSelected, origin);
        }
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

/// The scene's commands for the palette and the Keyboard page.
pub(crate) fn install(b: &mut ShellBuilder) {
    for (id, group, label, icon, keys) in SCENE_COMMANDS {
        let mut c = UiCommand::new(id, group, label, icon, move |cx: &mut DocCx<'_>| run_scene_command(id, cx.editor, cx.ctx)).enabled(|editor, active| active == crate::LEVEL && scene(editor).is_some());
        if !keys.is_empty() {
            c = c.keys(keys);
        }
        b.command(c);
    }
}

/// What the menus ask of the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuPicks {
    pub maximize: bool,
}

/// Draws Scene, Build and View.
pub fn draw(bar: &mut MenuBar<'_, '_>, cx: &mut DocCx<'_>, maximized: bool) -> MenuPicks {
    let editor = cx.editor.clone();
    let ctx = cx.ctx.clone();
    let mut picks = MenuPicks::default();
    let has_scene = scene(&editor).is_some();
    bar.menu("Scene", |ui| scene_menu(ui, &editor, &ctx, has_scene));
    let builds: Vec<&toolbar::LevelAction> = toolbar::LEVEL_ACTIONS.iter().filter(|a| a.group == 1 && editor.registry().has(a.command)).collect();
    let more_builds = BUILD_ROWS.iter().any(|(_, _, c, _)| editor.registry().has(c));
    if !builds.is_empty() || more_builds {
        bar.menu("Build", |ui| build_menu(ui, &editor, &builds));
    }
    bar.menu("View", |ui| {
        gz_ui_viewport::view_menu(ui, cx);
        ui.separator();
        if menu_item(ui, "Maximize Viewport", MenuItem { keys: "F11", selected: maximized, ..MenuItem::default() }) {
            picks.maximize = true;
        }
    });
    picks
}

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, editor: &Arc<Editor>, ctx: &Context, id: &str, label: &str, icon: Icon, default: &str, enabled: bool) {
    let keys = keys_now(editor, id, default);
    if menu_item(ui, label, MenuItem { icon, keys: &keys, enabled, ..MenuItem::default() }) {
        run_scene_command(id, editor, ctx);
    }
}

fn scene_menu(ui: &mut Ui, editor: &Arc<Editor>, ctx: &Context, has_scene: bool) {
    row(ui, editor, ctx, "COMMAND_SHOW_OBJECTLIST", "Find in World Outliner", Icon::Search, "Ctrl+F", true);
    ui.separator();
    let any = scene(editor).is_some_and(|s| !s.store.snapshot().selection.is_empty());
    if menu_item(ui, "Mark Selection as Mod Content", MenuItem { enabled: any, ..MenuItem::default() }) {
        mark(editor, true);
    }
    if menu_item(ui, "Mark Selection as Base Level", MenuItem { enabled: any, ..MenuItem::default() }) {
        mark(editor, false);
    }
    ui.separator();
    for (id, _, label, icon, keys) in SCENE_COMMANDS.iter().filter(|c| c.1 == "Selection" || c.1 == "Visibility") {
        row(ui, editor, ctx, id, label, *icon, keys, has_scene);
    }
    ui.separator();
    let can = scene(editor).is_some_and(|s| s.recovery.can_snapshot(true).is_ok());
    if menu_item(ui, "Create Recovery Snapshot", MenuItem { enabled: can, ..MenuItem::default() }) {
        let mut args = Args::new();
        args.insert("action".into(), Value::String("snapshot".into()));
        editor.post("recovery", args);
    }
}

/// `MarkSelection`: the build ships what is mod content and leaves the base level out.
fn mark(editor: &Arc<Editor>, mod_content: bool) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let mut args = Args::new();
        args.insert("authored".into(), Value::Bool(mod_content));
        let reply = ed.call("mark_authored", args, ed.ui_origin()).await;
        let n = reply.get("changed").and_then(Value::as_u64).unwrap_or(0);
        ed.log.info(if mod_content { format!("* Mod content: {n} object(s) marked.") } else { format!("* Base level: {n} object(s) unmarked.") });
    });
}

fn build_menu(ui: &mut Ui, editor: &Arc<Editor>, builds: &[&toolbar::LevelAction]) {
    let busy = toolbar::running(editor);
    for a in builds {
        if menu_item(ui, a.name, MenuItem { icon: a.icon, enabled: !busy, ..MenuItem::default() }) {
            toolbar::run(editor, a, Args::new());
        }
    }
    for sub in ["Make", "Compile"] {
        let rows: Vec<_> = BUILD_ROWS.iter().filter(|(s, _, c, _)| *s == sub && editor.registry().has(c)).collect();
        if rows.is_empty() {
            continue;
        }
        if !builds.is_empty() || sub == "Compile" {
            ui.separator();
        }
        local_submenu(ui, sub, Icon::None, !busy, |ui| {
            for (_, label, command, arg) in rows {
                if menu_item(ui, label, MenuItem::default()) {
                    let mut args = Args::new();
                    if !arg.is_empty() {
                        args.insert("mode".into(), Value::String((*arg).into()));
                    }
                    editor.post(command, args);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique() {
        let mut ids: Vec<&str> = SCENE_COMMANDS.iter().map(|c| c.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SCENE_COMMANDS.len());
    }
}
