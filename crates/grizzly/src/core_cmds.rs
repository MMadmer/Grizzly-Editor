//! Commands that belong to no area: the editor answering for itself.

use gz_editor::{ArgsExt, CmdError, Registry, json};

const AREA: &str = "core";

pub fn register(reg: &mut Registry) {
    reg.add_sync(AREA, "ping", |_, _, _| Ok(json!({ "name": "Grizzly Editor", "version": "1.0" })));

    // Without a window there is one thing left of window_control: closing the editor. The window
    // registers the full command over this one.
    reg.add_sync(AREA, "window_control", |editor, args, _| match args.str_or("action", "") {
        "close" => {
            editor.refuse_unsaved(args.flag("discard"))?;
            editor.request_quit();
            Ok(json!({ "closing": true }))
        }
        "" => Err(CmdError::invalid("missing 'action' argument")),
        other => Err(CmdError::invalid(format!("'{other}' needs a window; this editor runs without one"))),
    });

    reg.add_sync(AREA, "jobs", |editor, _, _| Ok(json!({ "jobs": editor.jobs.snapshot() })));
}
