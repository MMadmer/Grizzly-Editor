//! A project handed over by a second start of the editor (a double click on a project file while
//! this editor runs): when unsaved work is in the way the window asks Save / Don't Save / Cancel,
//! the way Switch Project asks, and opens the project after the answer.

use std::sync::Arc;

use gz_editor::{Args, Editor};
use gz_ui::{Answer, Asker};
use serde_json::json;

/// Hangs the question on the project area: `project.open from_launch=true` asks it.
pub fn install_hooks(editor: &Arc<Editor>) {
    gz_project::ProjectHooks::of(editor).provide_launch_prompt(|ed, folder| {
        let Ok(queue) = gz_app::UiQueue::of(ed) else { return false };
        let what: Vec<String> = ed.dirty_docs().into_iter().map(|d| d.name).collect();
        let (ed, folder) = (ed.clone(), folder.to_owned());
        queue.post(move |_shell, ctx| {
            let asker = if ed.flags.unattended { Asker::Unattended } else { Asker::Ui };
            let then = {
                let ed = ed.clone();
                move |answer: Answer| {
                    let key = match answer {
                        Answer::Yes => "save",
                        Answer::No => "discard",
                        Answer::Cancel => return,
                    };
                    let mut args = Args::new();
                    args.insert("path".to_owned(), json!(folder));
                    args.insert(key.to_owned(), json!(true));
                    ed.post("project.open", args);
                }
            };
            if let Some(line) = gz_ui::dialog::unsaved_prompt(ctx, asker, "open_project", &what, then) {
                ed.log.plain(line);
            }
        });
        true
    });
}
