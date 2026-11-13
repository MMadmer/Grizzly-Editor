//! How the surfaces run commands and system dialogs: off the UI thread, the answer posted back to
//! the start of a frame. The window and MCP so share one code path.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gz_app::UiQueue;
use gz_editor::{Args, Editor, Value};
use gz_ui::Asker;
use gz_ui::file_dialog::FileRequest;

/// Runs `name` with `args` as the window would (its origin) and hands the reply to `then` on the
/// UI thread at the start of the next frame.
pub(crate) fn call_then(
    editor: &Arc<Editor>,
    name: &'static str,
    args: Args,
    then: impl FnOnce(&Value, &egui::Context) + Send + 'static,
) {
    let ed = editor.clone();
    let origin = editor.ui_origin();
    let _task = editor.exec.spawn(async move {
        let reply = ed.call(name, args, origin).await;
        post_ui(&ed, move |ctx| then(&reply, ctx));
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

/// Asks for a frame: what a task left in a surface's inbox is drawn.
pub(crate) fn wake_ui(editor: &Editor) {
    post_ui(editor, |_| {});
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

/// Opens a tab by its kind through the shell's own opener (`ui.tab action=open`), from anywhere.
pub(crate) fn open_tab(editor: &Arc<Editor>, kind: &str) {
    let ed = editor.clone();
    let kind = kind.to_owned();
    let a = args([("action", s("open")), ("kind", s(&kind))]);
    let _task = editor.exec.spawn(async move {
        let reply = ed.call("ui.tab", a, ed.ui_origin()).await;
        if !ok(&reply) {
            ed.log.warn(format!("the {kind} tab did not open: {}", error_text(&reply)));
        }
    });
}

/// Opens a folder in Explorer.
pub(crate) fn open_folder(path: &str) {
    if !path.is_empty() {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
}

/// One system dialog at a time: a second click while one is up does nothing.
#[derive(Default)]
pub(crate) struct Picking(AtomicBool);

impl Picking {
    pub(crate) fn busy(editor: &Editor) -> bool {
        editor.services.get::<Picking>().is_some_and(|p| p.0.load(Ordering::SeqCst))
    }
}

/// What a system dialog picks.
pub(crate) enum Pick {
    Folder,
    /// A picture the thumbnail is made of.
    Picture,
}

/// Opens the system's folder or picture dialog on a thread of its own (it blocks while the
/// author picks), starting in `start` when that is a folder; `done` gets the path picked on the
/// UI thread, nothing when the author cancels.
pub(crate) fn pick_path(
    editor: &Arc<Editor>,
    what: Pick,
    title: &'static str,
    start: &str,
    done: impl FnOnce(String, &egui::Context) + Send + 'static,
) {
    let Ok(queue) = UiQueue::of(editor) else { return };
    let busy = editor.services.get_or_init(Picking::default);
    if busy.0.swap(true, Ordering::SeqCst) {
        return;
    }
    let release = Release(busy, queue.clone());
    let mut request = match what {
        Pick::Folder => FileRequest::folder(title),
        Pick::Picture => FileRequest::open(title).filter("Pictures", &["png", "jpg", "jpeg", "tga", "bmp"]),
    };
    if !start.is_empty() {
        request = request.in_folder(start);
    }
    let asker = if editor.flags.unattended { Asker::Unattended } else { Asker::Ui };
    let picked = gz_ui::file_dialog::pick(None, asker, request, move |path| {
        drop(release);
        let text = path.display().to_string();
        queue.post(move |_, ctx| {
            done(text, ctx);
            ctx.request_repaint();
        });
    });
    if let Err(line) = picked {
        editor.log.plain(line);
    }
}

/// Frees the one dialog's place when the dialog's answer is dropped, picked or not, and draws a
/// frame for the button that waited.
struct Release(Arc<Picking>, Arc<UiQueue>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::SeqCst);
        self.1.post(|_, ctx| ctx.request_repaint());
    }
}

/// Logs the lines a kit question or result returned: `!` an error, `~` a warning, `*` a note.
pub(crate) fn log_lines(editor: &Editor, lines: Option<String>) {
    let Some(lines) = lines else { return };
    for line in lines.lines() {
        if line.starts_with('!') {
            editor.log.error(line.to_owned());
        } else if line.starts_with('~') {
            editor.log.warn(line.to_owned());
        } else {
            editor.log.info(line.to_owned());
        }
    }
}
