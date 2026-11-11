//! The system's folder dialog, off the UI thread (the kit's `file_dialog`): it blocks its own
//! thread while the author picks, and the folder comes back to the start of a frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gz_app::UiQueue;
use gz_editor::Editor;
use gz_ui::Asker;
use gz_ui::file_dialog::FileRequest;

/// One folder dialog at a time: a second click while one is up does nothing.
#[derive(Default)]
struct Picking(AtomicBool);

/// Frees the one dialog's place when the dialog's answer is dropped, picked or not.
struct Release(Arc<Picking>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::SeqCst);
    }
}

/// Opens the folder dialog titled `title`, starting in `start` when that is set. `done` gets the
/// folder picked (nothing when the author cancels) on the UI thread. Under `-nodlg` nothing
/// opens.
pub(crate) fn pick_folder(
    editor: &Arc<Editor>,
    title: &'static str,
    start: &str,
    done: impl FnOnce(String, &egui::Context) + Send + 'static,
) {
    let Ok(queue) = UiQueue::of(editor) else { return };
    let busy = editor.services.get_or_init(Picking::default);
    if busy.0.swap(true, Ordering::SeqCst) {
        return;
    }
    let release = Release(busy);
    let mut request = FileRequest::folder(title);
    if !start.is_empty() {
        request = request.in_folder(start);
    }
    let asker = if editor.flags.unattended { Asker::Unattended } else { Asker::Ui };
    let picked = gz_ui::file_dialog::pick(None, asker, request, move |folder| {
        drop(release);
        let text = folder.display().to_string();
        queue.post(move |_, ctx| {
            done(text, ctx);
            ctx.request_repaint();
        });
    });
    if let Err(line) = picked {
        editor.log.plain(line);
    }
}
