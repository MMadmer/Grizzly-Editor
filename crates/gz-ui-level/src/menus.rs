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
use gz_ui::{Icon, MenuItem, menu_item};

use crate::doc::{Act, post_to_level};
use crate::toolbar;

/// The scene's commands: id, group, label, icon, default keys.
pub const SCENE_COMMANDS: [(&str, &str, &str, Icon, &str); 11] = [
    ("COMMAND_UNDO", "Edit", "Undo", Icon::Undo, "Ctrl+Z"),
    ("COMMAND_REDO", "Edit", "Redo", Icon::Redo, "Ctrl+Y"),
    ("COMMAND_DELETE_SELECTION", "Edit", "Delete", Icon::Delete, "Delete"),
    ("COMMAND_SHOW_OBJECTLIST", "Scene", "Find in World Outliner", Icon::Search, "Ctrl+W"),
    ("COMMAND_SELECT_ALL", "Selection", "Select All", Icon::Select, "Ctrl+A"),
    ("COMMAND_DESELECT_ALL", "Selection", "Unselect All", Icon::Select, ""),
    ("COMMAND_INVERT_SELECTION_ALL", "Selection", "Invert Selection", Icon::Select, "Ctrl+I"),
    ("COMMAND_HIDE_SEL", "Visibility", "Hide Selected", Icon::EyeOff, ""),
    ("COMMAND_HIDE_UNSEL", "Visibility", "Hide Unselected", Icon::EyeOff, ""),
    ("COMMAND_HIDE_ALL", "Visibility", "Hide All", Icon::EyeOff, ""),
    ("COMMAND_UNHIDE_ALL", "Visibility", "Unhide All", Icon::Eye, ""),
];

/// The Build menu's rows past the toolbar's builds: submenu, label, command. The AI map, the
/// game graph and the spawn are built in the editor's process, so no row starts a program.
const BUILD_ROWS: [(&str, &str, &str); 8] = [
    ("Make", "Make All...", "make_all"),
    ("Make", "Make Game...", "make_game"),
    ("Make", "Make Details...", "make_details"),
    ("Make", "Make HOM...", "make_hom"),
    ("Make", "Make SOM...", "make_som"),
    ("Make", "Make AI Map...", "make_aimap"),
    ("Compile", "Geometry & Light", "compile_geometry"),
    ("Compile", "Detail Object Light", "compile_details"),
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
    // the AI map, the detail slots and the wallmarks keep their own selection
    if crate::tools_bulk::menu_command(id, editor, ctx, &snap) {
        return;
    }
    match id {
        "COMMAND_UNDO" => editor.post("undo", Args::new()),
        "COMMAND_REDO" => editor.post("redo", Args::new()),
        // `CommandDeleteSelection`: asks nothing, one undo step
        "COMMAND_DELETE_SELECTION" => {
            svc.store.post(SceneCmd::RemoveSelection { class: Some(class), label: "Delete".into() }, origin)
        }
        "COMMAND_SELECT_ALL" => {
            svc.store.post(SceneCmd::SelectClass { class: Some(class), select: true }, origin)
        }
        "COMMAND_DESELECT_ALL" => {
            svc.store.post(SceneCmd::SelectClass { class: Some(class), select: false }, origin)
        }
        "COMMAND_INVERT_SELECTION_ALL" => {
            let ids = snap.of_class(class).filter(|o| o.flags.visible).map(|o| o.id).collect();
            svc.store.post(SceneCmd::Select { ids, mode: SelectMode::Toggle }, origin);
        }
        // `ShowObjects(false, class, ...)`: the selected, the unselected or every object of the class
        "COMMAND_HIDE_SEL" | "COMMAND_HIDE_UNSEL" | "COMMAND_HIDE_ALL" | "COMMAND_UNHIDE_ALL" => {
            let visible = id == "COMMAND_UNHIDE_ALL";
            let ids: Vec<_> = snap
                .of_class(class)
                .filter(|o| match id {
                    "COMMAND_HIDE_SEL" => snap.is_selected(o.id),
                    "COMMAND_HIDE_UNSEL" => !snap.is_selected(o.id),
                    _ => true,
                })
                .map(|o| o.id)
                .collect();
            if !ids.is_empty() {
                svc.store.post(SceneCmd::Show { ids, visible }, origin);
            }
        }
        _ => {}
    }
}

/// The scene's commands for the palette and the Keyboard page.
pub(crate) fn install(b: &mut ShellBuilder) {
    for (id, group, label, icon, keys) in SCENE_COMMANDS {
        let mut c = UiCommand::new(id, group, label, icon, move |cx: &mut DocCx<'_>| {
            run_scene_command(id, cx.editor, cx.ctx)
        })
        .enabled(|editor, active| active == crate::LEVEL && scene(editor).is_some());
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
    let builds: Vec<&toolbar::LevelAction> =
        toolbar::LEVEL_ACTIONS.iter().filter(|a| a.group == 1 && editor.registry().has(a.command)).collect();
    let more_builds = BUILD_ROWS.iter().any(|(_, _, c)| editor.registry().has(c));
    if !builds.is_empty() || more_builds || has_scene {
        bar.menu("Build", |ui| build_menu(ui, &editor, &builds));
    }
    bar.menu("View", |ui| {
        gz_ui_viewport::view_menu(ui, cx);
        ui.separator();
        if menu_item(
            ui,
            "Maximize Viewport",
            MenuItem { keys: "F11", selected: maximized, ..MenuItem::default() },
        ) {
            picks.maximize = true;
        }
    });
    picks
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    ctx: &Context,
    id: &str,
    label: &str,
    icon: Icon,
    default: &str,
    enabled: bool,
) {
    let keys = keys_now(editor, id, default);
    if menu_item(ui, label, MenuItem { icon, keys: &keys, enabled, ..MenuItem::default() }) {
        run_scene_command(id, editor, ctx);
    }
}

fn scene_menu(ui: &mut Ui, editor: &Arc<Editor>, ctx: &Context, has_scene: bool) {
    let item = |ui: &mut Ui, label: &str, id: &str, enabled: bool| {
        let keys = keys_now(editor, id, "");
        menu_item(ui, label, MenuItem { keys: &keys, enabled, ..MenuItem::default() })
    };
    if item(ui, "Validate", "COMMAND_VALIDATE_SCENE", has_scene) {
        crate::level_cmds::validate(editor, ctx);
    }
    if item(ui, "Summary Info", "COMMAND_SHOW_SCENE_SUMMARY", has_scene) {
        crate::level_cmds::summary(editor);
    }
    row(ui, editor, ctx, "COMMAND_SHOW_OBJECTLIST", "Find in World Outliner", Icon::Search, "Ctrl+W", true);
    if item(ui, "Clear Debug Draw", "COMMAND_CLEAR_DEBUG_DRAW", has_scene) {
        crate::level_cmds::clear_debug_draw(editor);
    }
    ui.separator();
    if item(ui, "Open Selection...", "COMMAND_LOAD_SELECTION", has_scene) {
        let _ = crate::level_cmds::open_selection(editor, ctx);
    }
    if item(ui, "Save Selection As...", "COMMAND_SAVE_SELECTION", has_scene) {
        let _ = crate::level_cmds::save_selection(editor, ctx);
    }
    if item(ui, "Export Scene as OBJ...", "", has_scene) {
        let _ = crate::level_cmds::export_obj(editor, ctx, false);
    }
    if item(ui, "Export Selection as OBJ...", "", has_scene) {
        let _ = crate::level_cmds::export_obj(editor, ctx, true);
    }
    if item(ui, "Rename Objects After References...", "COMMAND_MULTI_RENAME_OBJECTS", has_scene) {
        crate::level_cmds::rename_objects(editor, ctx);
    }
    ui.separator();
    let any = scene(editor).is_some_and(|s| !s.store.snapshot().selection.is_empty());
    if menu_item(ui, "Mark Selection as Mod Content", MenuItem { enabled: any, ..MenuItem::default() }) {
        mark(editor, true);
    }
    if menu_item(ui, "Mark Selection as Base Level", MenuItem { enabled: any, ..MenuItem::default() }) {
        mark(editor, false);
    }
    ui.separator();
    for (id, _, label, icon, keys) in
        SCENE_COMMANDS.iter().filter(|c| c.1 == "Selection" || c.1 == "Visibility")
    {
        row(ui, editor, ctx, id, label, *icon, keys, has_scene);
    }
    ui.separator();
    // `etfPickAnyClass`: a click selects an object of any class and switches the tool to it
    let vp = gz_ui_viewport::Viewport::of(editor);
    let any = vp.read(|s| s.settings.pick_any_class);
    if menu_item(ui, "Pick Any Class", MenuItem { selected: any, ..MenuItem::default() }) {
        vp.with(|s| s.settings.pick_any_class = !any);
    }
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
        ed.log.info(if mod_content {
            format!("* Mod content: {n} object(s) marked.")
        } else {
            format!("* Base level: {n} object(s) unmarked.")
        });
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
        let rows: Vec<_> =
            BUILD_ROWS.iter().filter(|(s, _, c)| *s == sub && editor.registry().has(c)).collect();
        if rows.is_empty() {
            continue;
        }
        if !builds.is_empty() || sub == "Compile" {
            ui.separator();
        }
        local_submenu(ui, sub, Icon::None, !busy, |ui| {
            for (_, label, command) in rows {
                let id = crate::level_cmds::make_id(command);
                let keys = id.map(|i| keys_now(editor, i, "")).unwrap_or_default();
                if menu_item(ui, label, MenuItem { keys: &keys, ..MenuItem::default() }) {
                    if sub == "Make" {
                        // `ConfirmLevelBuild`: the level folder named first
                        crate::level_cmds::ask_make(editor, ui.ctx(), command);
                    } else {
                        crate::level_cmds::compile(editor, ui.ctx(), command);
                    }
                }
            }
        });
    }
    ui.separator();
    let has_scene = scene(editor).is_some();
    if menu_item(ui, "Import Error List...", MenuItem { enabled: has_scene, ..MenuItem::default() }) {
        let _ = crate::level_cmds::import_errors(editor, ui.ctx());
    }
    if menu_item(ui, "Export Error List...", MenuItem { enabled: has_scene, ..MenuItem::default() }) {
        let _ = crate::level_cmds::export_errors(editor, ui.ctx());
    }
    if menu_item(ui, "Clear Error List", MenuItem { enabled: has_scene, ..MenuItem::default() }) {
        crate::level_cmds::clear_debug_draw(editor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_commands_share_a_default_key() {
        // the scene's, the viewport's, the tools', the builds' and the file commands' defaults
        let mut keys: Vec<(String, &str)> = Vec::new();
        for (id, _, _, _, k) in SCENE_COMMANDS {
            if !k.is_empty() {
                keys.push((gz_app::Chord::parse(k).map(|c| c.label()).unwrap_or_default(), id));
            }
        }
        for b in gz_ui_viewport::keys::BINDINGS {
            if let Some(c) = b.keys {
                keys.push((c.label(), b.id));
            }
        }
        for (id, k) in crate::level_cmds::DEFAULT_KEYS {
            keys.push((gz_app::Chord::parse(k).map(|c| c.label()).unwrap_or_default(), id));
        }
        // a command listed twice (the scene's and the viewport's own row of it) is one command
        keys.sort();
        keys.dedup();
        for w in keys.windows(2) {
            assert!(w[0].0 != w[1].0, "{} and {} both default to {}", w[0].1, w[1].1, w[0].0);
        }
    }

    #[test]
    fn command_ids_are_unique() {
        let mut ids: Vec<&str> = SCENE_COMMANDS.iter().map(|c| c.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SCENE_COMMANDS.len());
    }
}
