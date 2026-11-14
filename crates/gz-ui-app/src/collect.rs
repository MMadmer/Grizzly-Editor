//! File > Project > Collect Library Assets...: everything the project's scenes still take from the
//! editor's library, copied into the project. It says what and how much first and asks once - the
//! kit's confirm, no danger: nothing is lost - then runs `project.collect`, whose job has a card in
//! the Task Center.

use std::sync::Arc;

use gz_app::{Action, DialogDesc, Enabled, ShellBuilder};
use gz_editor::{Args, Editor, SmolStr};
use gz_ui::dialog::{Answer, ConfirmSpec, confirm, notice};
use gz_ui::{Asker, Icon, Tone};
use serde_json::{Value, json};

use crate::reply::{call_then, error_text, ok};

/// The dialog's MCP id (`ui.dialog action=open id=collect_library`).
pub const DIALOG: &str = "collect_library";
/// The File > Project row.
pub const ACTION: &str = "project.collect_library";

/// `12.3 MB`.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} bytes") } else { format!("{v:.1} {}", UNITS[unit]) }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.dialog(DialogDesc {
        id: SmolStr::new(DIALOG),
        window: SmolStr::new("Collect Library Assets###dlg.collect_library"),
        needs_project: true,
        open: Box::new(|cx| {
            ask(cx.editor);
            Ok(())
        }),
        // the question is the kit's queued confirm; nothing of its own is drawn
        draw: Box::new(|_, _| {}),
    });
    b.action(
        Action::new(ACTION, "Collect Library Assets...", "File", "ui.dialog")
            .icon(Icon::Package)
            .args(|_| gz_app::args(&[("action", "open"), ("id", DIALOG)]))
            .enabled(|cx| Enabled::when(cx.editor.project_opt().is_some(), "Open a project first.")),
    );
    b.submenu_action("File", "Project", 55, 0, ACTION, None);
}

fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

/// Works out what the collect copies, then asks.
fn ask(editor: &Arc<Editor>) {
    let mut dry = Args::new();
    dry.insert("dry_run".to_owned(), json!(true));
    let ed = editor.clone();
    call_then(editor, "project.collect", dry, move |reply, ctx| {
        if !ok(reply) {
            if let Some(line) =
                notice(ctx, asker(&ed), Tone::Error, "Collect Library Assets", &error_text(reply))
            {
                ed.log.finding(&line);
            }
            return;
        }
        let files = reply.get("files").and_then(Value::as_u64).unwrap_or(0);
        let bytes = reply.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        let refs: Vec<String> = reply
            .get("refs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        if files == 0 {
            let text = if refs.is_empty() {
                "The project takes nothing from the editor's library: it is complete by itself."
            } else {
                "What the project takes from the library is not in the library either: the Problems name it."
            };
            if let Some(line) = notice(ctx, asker(&ed), Tone::Info, "Collect Library Assets", text) {
                ed.log.finding(&line);
            }
            return;
        }
        let spec = ConfirmSpec {
            id: DIALOG.to_owned(),
            title: "Collect Library Assets".to_owned(),
            headline: format!(
                "Copy {files} {} ({}) from the editor's library into the project?",
                if files == 1 { "file" } else { "files" },
                size_text(bytes)
            ),
            total: refs.len(),
            items: refs,
            consequence: String::new(),
            action: "Collect".to_owned(),
            destructive: false,
        };
        let go = ed.clone();
        let line = confirm(ctx, asker(&ed), spec, move |answer| {
            if answer == Answer::Yes {
                go.post("project.collect", Args::new());
            }
        });
        if let Some(line) = line {
            ed.log.finding(&line);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_read_as_the_author_reads_them() {
        assert_eq!(super::size_text(900), "900 bytes");
        assert_eq!(super::size_text(1536), "1.5 KB");
        assert_eq!(super::size_text(5 * 1024 * 1024), "5.0 MB");
    }
}
