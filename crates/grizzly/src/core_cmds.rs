//! Commands that belong to no area: the editor answering for itself.

use gz_editor::{NoParams, Registry, Spec, json};

pub fn register(reg: &mut Registry) {
    reg.cmd_sync(
        Spec::read("ping", "Says the editor is there: its name and the version it was built as."),
        |_, _: &NoParams, _| Ok(json!({ "name": "Grizzly Editor", "version": env!("CARGO_PKG_VERSION") })),
    );
}
