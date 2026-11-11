//! Mod > Bake Scene Layer (`MenuMod`, `EditorModScene::RequestExportSpawn` / `RequestExportXCForm`
//! / `RequestExportOGF` / `RequestExportCut`): each row bakes the selection of the open scene
//! into a level delta of the module at once - the four are faces of `mod_export_spawn`,
//! `mod_export_xcform`, `mod_export_ogf_probe` and `mod_export_cut` - and reports in the kit's
//! one Result dialog, `dlg.bake_result`, with Open Folder.

use std::sync::Arc;

use gz_app::{CppAction, CppCommand, DocCx, MenuContent, MenuEntry, ShellBuilder, UiCommand};
use gz_editor::{Args, Editor, SmolStr, Value};
use gz_ui::Icon;
use gz_ui::Tone;
use gz_ui::dialog::{ResultAction, ResultSpec};

use crate::call;

/// The four bakes: the command id, the C++-table id it is listed under, the menu row, the face's
/// name, the editor command and what a failure is called.
const BAKES: [(&str, u32, &str, &str, &str, &str); 4] = [
    ("COMMAND_MOD_BAKE_SPAWN", 128, "Spawn Layer...", "Bake Spawn Layer", "mod_export_spawn", "The spawn layer was not written"),
    (
        "COMMAND_MOD_BAKE_XCFORM",
        129,
        "Collision Overlay...",
        "Bake Collision Overlay",
        "mod_export_xcform",
        "The collision overlay was not written",
    ),
    (
        "COMMAND_MOD_BAKE_OGF",
        130,
        "Visual Overlay...",
        "Bake Visual Overlay",
        "mod_export_ogf_probe",
        "The visual overlay was not written",
    ),
    ("COMMAND_MOD_BAKE_CUT", 131, "Level Cut...", "Bake Level Cut", "mod_export_cut", "The level cut was not written"),
];

fn num(reply: &Value, key: &str) -> i64 {
    reply.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(reply: &Value, key: &str) -> String {
    reply.get(key).and_then(Value::as_str).unwrap_or("").replace('/', "\\")
}

/// `Num(n) + " object(s) ..."`.
fn count_line(n: i64, rest: &str) -> Option<String> {
    (n != 0).then(|| format!("{n} {rest}"))
}

/// What a bake's answer says, as `BakeDone` / `BakeFailed` show it.
fn report(command: &str, failed: &str, reply: &Value) -> (ResultSpec, String) {
    if !call::ok(reply) {
        let spec = ResultSpec::new(
            "bake_result",
            "Bake Failed",
            Tone::Error,
            &format!("{failed}: {}", call::error_text(reply)),
        );
        return (spec, String::new());
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let (headline, folder) = match command {
        "mod_export_spawn" => {
            let path = text(reply, "path");
            rows.push(("File".into(), path.clone()));
            rows.push(("Skipped".into(), num(reply, "skipped").to_string()));
            rows.push(("Player Starts".into(), num(reply, "starts").to_string()));
            rows.push(("Way paths".into(), num(reply, "paths").to_string()));
            rows.push(("NPC orders".into(), num(reply, "orders").to_string()));
            warnings.extend(count_line(
                num(reply, "scaled"),
                "object(s) have a scale other than 1: the spawn layer carries no scale.",
            ));
            // a stash keeps its ghost model unless the author picks another one
            warnings.extend(count_line(
                num(reply, "invisible"),
                "object(s) use a model the engine draws nothing for: they work but stay invisible in game. Give them a real mesh in Model > Visual (the log names them).",
            ));
            if let Some(list) = reply.get("warnings").and_then(Value::as_array) {
                warnings.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            (format!("The spawn layer holds {} op(s) of the selection.", num(reply, "exported")), parent(&path))
        }
        "mod_export_xcform" => {
            let path = text(reply, "path");
            rows.push(("File".into(), path.clone()));
            rows.push(("Vertices".into(), num(reply, "vertices").to_string()));
            rows.push(("Triangles".into(), num(reply, "tris").to_string()));
            rows.push(("Materials".into(), num(reply, "materials").to_string()));
            ("The collision overlay of the selection is written.".to_owned(), parent(&path))
        }
        "mod_export_ogf_probe" => {
            let dir = text(reply, "dir");
            rows.push(("Folder".into(), dir.clone()));
            rows.push(("Models".into(), num(reply, "exported").to_string()));
            rows.push(("Registry".into(), "overlay_visuals.ltx".into()));
            warnings.extend(count_line(
                num(reply, "skipped_dynamic"),
                "dynamic object(s) skipped: an overlay holds static visuals only.",
            ));
            warnings.extend(count_line(
                num(reply, "skipped_failed"),
                "object(s) failed to export (the log names them).",
            ));
            ("The visual overlay of the selection is written.".to_owned(), dir)
        }
        _ => {
            let path = text(reply, "path");
            rows.push(("File".into(), path.clone()));
            rows.push(("Volumes".into(), num(reply, "cuts").to_string()));
            (
                "Base collision inside the boxes is removed and base visuals fully inside them are detached. Put your own geometry in with the other layers.".to_owned(),
                parent(&path),
            )
        }
    };
    let tone = if warnings.is_empty() { Tone::Success } else { Tone::Warning };
    let mut spec = ResultSpec::new("bake_result", "Scene Layer Baked", tone, &headline);
    spec.rows = rows;
    spec.warnings = warnings;
    (spec, folder)
}

/// The folder of a file (`OpenFolderOf` with `is_file`).
fn parent(path: &str) -> String {
    path.rfind(['\\', '/']).map_or_else(|| path.to_owned(), |i| path.get(..i).unwrap_or("").to_owned())
}

/// Runs a bake as the window does and shows what it came to.
fn bake(editor: &Arc<Editor>, command: &'static str, failed: &'static str) {
    let ed = editor.clone();
    call::call_then(editor, command, Args::new(), move |reply, ctx| {
        let (mut spec, folder) = report(command, failed, reply);
        if !folder.is_empty() {
            spec.action = Some(ResultAction {
                label: "Open Folder".to_owned(),
                run: Arc::new(move || call::open_folder(&folder)),
            });
        }
        crate::build::show_result(&ed, ctx, spec);
    });
}

fn has_project(editor: &Editor, _tab: &str) -> bool {
    editor.project_opt().is_some()
}

pub(crate) fn install(b: &mut ShellBuilder) {
    let (group, order) = crate::menu::mod_menu_slot(crate::menu::BAKE_SCENE_LAYER).unwrap_or((1, 3));
    for (k, (id, cpp_id, row, label, command, failed)) in BAKES.into_iter().enumerate() {
        b.command(
            UiCommand::new(id, "Mod", label, Icon::Level, move |cx: &mut DocCx<'_>| bake(cx.editor, command, failed))
                .enabled(has_project),
        );
        b.cpp_command(
            CppCommand::new(cpp_id, id, &format!("Mod\\Bake Scene Layer\\{}", row.trim_end_matches('.')), CppAction::Ui)
                .face("Mod", label, Icon::Level),
        );
        b.menu_entry(MenuEntry {
            menu: SmolStr::new("Mod"),
            submenu: Some(SmolStr::new(crate::menu::BAKE_SCENE_LAYER)),
            group,
            order: order * 10 + i32::try_from(k).unwrap_or(0),
            content: MenuContent::Command { command: SmolStr::new(id), label: Some(SmolStr::new(row)) },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_editor::json;

    #[test]
    fn a_spawn_bake_reads_like_bake_done() {
        let reply = json!({ "ok": true, "exported": 3, "skipped": 1, "scaled": 2, "invisible": 0, "starts": 1, "paths": 0, "orders": 0, "path": "C:/m/spawn/l01.xspawn", "warnings": ["w"] });
        let (spec, folder) = report("mod_export_spawn", "x", &reply);
        assert_eq!(spec.headline, "The spawn layer holds 3 op(s) of the selection.");
        assert_eq!(spec.tone, Tone::Warning);
        assert_eq!(spec.warnings.len(), 2);
        assert_eq!(folder, "C:\\m\\spawn");
    }

    #[test]
    fn a_refusal_is_bake_failed() {
        let reply = json!({ "ok": false, "error": "no scene" });
        let (spec, folder) = report("mod_export_cut", "The level cut was not written", &reply);
        assert_eq!(spec.title, "Bake Failed");
        assert_eq!(spec.headline, "The level cut was not written: no scene");
        assert!(folder.is_empty());
    }
}
