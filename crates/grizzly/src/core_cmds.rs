//! Commands that belong to no area: the editor answering for itself.

use gz_editor::{NoParams, Registry, Spec, json};

pub fn register(reg: &mut Registry) {
    reg.cmd_sync(
        Spec::read("ping", "Says the editor is there: its name and the version it was built as."),
        |editor, _: &NoParams, _| {
            let mut answer = json!({ "name": "Grizzly Editor", "version": env!("CARGO_PKG_VERSION") });
            // a QA run says so: a harness refuses to drive an editor that could reach a real service
            if editor.flags.qa_github {
                answer["qa"] = json!(true);
            }
            Ok(answer)
        },
    );
}
