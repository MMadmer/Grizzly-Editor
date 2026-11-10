//! How the surfaces run a command: by its wire name, off the UI thread, with the reply posted
//! back to the start of a frame. The window and MCP so share one code path.

use std::sync::Arc;

use gz_app::UiQueue;
use gz_editor::{Args, Editor, Value};

/// Runs `name` with `args` as the window would (its origin), and hands the whole reply to
/// `then` on the UI thread at the start of the next frame.
pub(crate) fn call_then(editor: &Arc<Editor>, name: &'static str, args: Args, then: impl FnOnce(&Value, &egui::Context) + Send + 'static) {
    let ed = editor.clone();
    let origin = editor.ui_origin();
    let _task = editor.exec.spawn(async move {
        let reply = ed.call(name, args, origin).await;
        if let Ok(queue) = UiQueue::of(&ed) {
            queue.post(move |_, ctx| {
                then(&reply, ctx);
                ctx.request_repaint();
            });
        }
    });
}

/// Runs `f` on the UI thread at the start of the next frame, when there is a window.
pub(crate) fn post_ui(editor: &Editor, f: impl FnOnce(&egui::Context) + Send + 'static) {
    if let Ok(queue) = UiQueue::of(editor) {
        queue.post(move |_, ctx| {
            f(ctx);
            ctx.request_repaint();
        });
    }
}

/// The reply said `ok`.
pub(crate) fn ok(reply: &Value) -> bool {
    reply.get("ok").and_then(Value::as_bool) == Some(true)
}

/// The reply's `error`, or a sentence when it has none.
pub(crate) fn error_text(reply: &Value) -> String {
    reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned()
}

/// Arguments from `(key, value)` pairs.
pub(crate) fn args<const N: usize>(pairs: [(&str, Value); N]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert(k.to_owned(), v);
    }
    a
}

/// A string argument.
pub(crate) fn s(text: &str) -> Value {
    Value::String(text.to_owned())
}
