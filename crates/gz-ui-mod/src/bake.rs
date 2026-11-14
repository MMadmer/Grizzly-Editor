//! Mod > Bake Scene Layer: each row bakes the selection of the open scene into a level delta of
//! the module at once - the four are actions over `level.export` (`kind=spawn`, `collision`,
//! `visuals`, `cut`) - and reports in the kit's one Result dialog, `dlg.bake_result`, with Open
//! Folder.

use std::sync::Arc;

use gz_app::{Action, ActionCx, ReplyCx, ShellBuilder};
use gz_editor::{Args, Value};
use gz_ui::Icon;
use gz_ui::Tone;
use gz_ui::dialog::{ResultAction, ResultSpec};

use crate::call;

/// The four bakes: the action, the menu row, the action's title, the layer (`level.export kind=`)
/// and what a failure is called.
const BAKES: [(&str, &str, &str, &str, &str); 4] = [
    ("mod.bake_spawn", "Spawn Layer...", "Bake Spawn Layer", "spawn", "The spawn layer was not written"),
    (
        "mod.bake_collision",
        "Collision Overlay...",
        "Bake Collision Overlay",
        "collision",
        "The collision overlay was not written",
    ),
    (
        "mod.bake_visuals",
        "Visual Overlay...",
        "Bake Visual Overlay",
        "visuals",
        "The visual overlay was not written",
    ),
    ("mod.bake_cut", "Level Cut...", "Bake Level Cut", "cut", "The level cut was not written"),
];

fn num(reply: &Value, key: &str) -> i64 {
    reply.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(reply: &Value, key: &str) -> String {
    reply.get(key).and_then(Value::as_str).unwrap_or("").replace('/', "\\")
}

/// `n` and what it counts, or nothing for zero.
fn count_line(n: i64, rest: &str) -> Option<String> {
    (n != 0).then(|| format!("{n} {rest}"))
}

/// What a bake's answer says, as the result shows it.
fn report(kind: &str, failed: &str, reply: &Value) -> (ResultSpec, String) {
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
    let (headline, folder) = match kind {
        "spawn" => {
            let path = text(reply, "path");
            rows.push(("File".into(), path.clone()));
            rows.push(("Skipped".into(), num(reply, "skipped").to_string()));
            rows.push(("Player Starts".into(), num(reply, "starts").to_string()));
            rows.push(("Way paths".into(), num(reply, "paths").to_string()));
            rows.push(("NPC orders".into(), num(reply, "orders").to_string()));
            rows.push(("Base objects removed".into(), num(reply, "removed").to_string()));
            warnings.extend(count_line(
                num(reply, "scaled"),
                "object(s) have a scale other than 1: the spawn layer carries no scale.",
            ));
            warnings.extend(count_line(
                num(reply, "missing"),
                "object(s) are not baked: their model exists neither in the game nor in the project.",
            ));
            // a stash keeps its ghost model unless the author picks another one
            warnings.extend(count_line(
                num(reply, "invisible"),
                "object(s) use a model the engine draws nothing for: they work but stay invisible in game. Give them a real mesh in Model > Visual (the log names them).",
            ));
            if let Some(list) = reply.get("warnings").and_then(Value::as_array) {
                warnings.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            (
                format!("The spawn layer holds {} op(s) of the selection.", num(reply, "exported")),
                parent(&path),
            )
        }
        "collision" => {
            let path = text(reply, "path");
            rows.push(("File".into(), path.clone()));
            rows.push(("Vertices".into(), num(reply, "vertices").to_string()));
            rows.push(("Triangles".into(), num(reply, "tris").to_string()));
            rows.push(("Materials".into(), num(reply, "materials").to_string()));
            ("The collision overlay of the selection is written.".to_owned(), parent(&path))
        }
        "visuals" => {
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

/// The folder of a file (with `is_file`).
fn parent(path: &str) -> String {
    path.rfind(['\\', '/']).map_or_else(|| path.to_owned(), |i| path.get(..i).unwrap_or("").to_owned())
}

/// Runs a bake as the window does and shows what it came to.
/// The kit's result of a bake, from its answer.
fn show_bake(r: &ReplyCx<'_>, n: usize) {
    let Some((_, _, _, kind, failed)) = BAKES.get(n) else { return };
    let (mut spec, folder) = report(kind, failed, r.reply);
    if !folder.is_empty() {
        spec.action = Some(ResultAction {
            label: "Open Folder".to_owned(),
            run: Arc::new(move || call::open_folder(&folder)),
        });
    }
    crate::build::show_result(r.editor, r.ctx, spec);
}

pub(crate) fn install(b: &mut ShellBuilder) {
    let (group, order) = crate::menu::mod_menu_slot(crate::menu::BAKE_SCENE_LAYER).unwrap_or((1, 3));
    let shows: [fn(&ReplyCx<'_>); 4] =
        [|r| show_bake(r, 0), |r| show_bake(r, 1), |r| show_bake(r, 2), |r| show_bake(r, 3)];
    let kinds: [fn(&ActionCx<'_>) -> Args; 4] = [
        |_| gz_app::actions::args(&[("kind", "spawn")]),
        |_| gz_app::actions::args(&[("kind", "collision")]),
        |_| gz_app::actions::args(&[("kind", "visuals")]),
        |_| gz_app::actions::args(&[("kind", "cut")]),
    ];
    for (k, (((id, row, label, _, _), show), kind)) in BAKES.into_iter().zip(shows).zip(kinds).enumerate() {
        b.action(
            Action::new(id, label, "Mod", "level.export")
                .args(kind)
                .icon(Icon::Level)
                .enabled(crate::menu::with_project)
                .reply(show),
        );
        b.submenu_action(
            "Mod",
            crate::menu::BAKE_SCENE_LAYER,
            group,
            order * 10 + i32::try_from(k).unwrap_or(0),
            id,
            Some(row),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_editor::json;

    #[test]
    fn a_spawn_bake_reads_like_bake_done() {
        let reply = json!({ "ok": true, "exported": 3, "skipped": 1, "scaled": 2, "invisible": 0, "starts": 1, "paths": 0, "orders": 0, "path": "C:/m/spawn/l01.xspawn", "warnings": ["w"] });
        let (spec, folder) = report("spawn", "x", &reply);
        assert_eq!(spec.headline, "The spawn layer holds 3 op(s) of the selection.");
        assert_eq!(spec.tone, Tone::Warning);
        assert_eq!(spec.warnings.len(), 2);
        assert_eq!(folder, "C:\\m\\spawn");
    }

    #[test]
    fn a_refusal_is_bake_failed() {
        let reply = json!({ "ok": false, "error": "no scene" });
        let (spec, folder) = report("cut", "The level cut was not written", &reply);
        assert_eq!(spec.title, "Bake Failed");
        assert_eq!(spec.headline, "The level cut was not written: no scene");
        assert!(folder.is_empty());
    }
}
