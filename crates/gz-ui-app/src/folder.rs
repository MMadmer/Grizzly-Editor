//! The system's folder dialog, off the UI thread: it blocks its own thread while the author
//! picks, and the folder comes back to the start of a frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gz_app::UiQueue;
use gz_editor::Editor;

/// One folder dialog at a time: a second click while one is up does nothing.
#[derive(Default)]
struct Picking(AtomicBool);

/// Opens the folder dialog titled `title`, starting in `start` when that is set. `done` gets the
/// folder picked (nothing when the author cancels) on the UI thread.
pub(crate) fn pick_folder(editor: &Arc<Editor>, title: &'static str, start: &str, done: impl FnOnce(String, &egui::Context) + Send + 'static) {
    let Ok(queue) = UiQueue::of(editor) else { return };
    let busy = editor.services.get_or_init(Picking::default);
    if busy.0.swap(true, Ordering::SeqCst) {
        return;
    }
    let start = start.to_owned();
    let flag = busy.clone();
    let spawned = std::thread::Builder::new().name("gz-ui-app.folder".to_owned()).spawn(move || {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if !start.is_empty() && std::path::Path::new(&start).is_dir() {
            dialog = dialog.set_directory(&start);
        }
        let picked = dialog.pick_folder();
        flag.0.store(false, Ordering::SeqCst);
        if let Some(folder) = picked {
            let text = folder.display().to_string();
            queue.post(move |_, ctx| {
                done(text, ctx);
                ctx.request_repaint();
            });
        }
    });
    if let Err(e) = spawned {
        busy.0.store(false, Ordering::SeqCst);
        editor.log.warn(format!("the folder dialog did not open: {e}"));
    }
}
