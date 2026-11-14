//! The three library tabs (docs/UI_DESIGN.md, section 6, "Libraries"): list | preview or
//! timeline | grid. Light Animations edits `gamedata\lanims.xr`, the Texture Library the
//! descriptions of `rawdata\textures` (and turns into New Textures for the import folder), the
//! Sound Library the descriptions of `rawdata\sounds`. Lists come from a walk on the IO pool;
//! edits live in a `Store` per library with undo and are saved explicitly.

pub mod keys;
pub mod lanim;
pub mod list;
pub mod sounds;
pub mod stores;
pub mod texpick;
pub mod texsync;
pub mod textures;

use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, Rect, ScrollArea, Ui, UiBuilder, pos2};
use gz_editor::{Args, Editor};
use gz_ui::palette;
use gz_ui::splitter::{splitter, splitter_tail};
use serde_json::{Map, Value};

/// pt: list | preview | properties.
pub const LIST_W: f32 = 260.0;
pub const GRID_W: f32 = 320.0;
pub const SIDE_MIN: f32 = 220.0;
pub const CANVAS_MIN: f32 = 400.0;

/// The library kinds, as `library kind=` and the tab ids name them.
pub const KINDS: [&str; 3] = ["light_animations", "texture_library", "sound_library"];

/// The panes of a library tab: the list and the properties keep their pt size, the preview takes
/// every resize. Returns the list, the preview and the properties.
pub fn panes(ui: &mut Ui, kind: &str) -> [Rect; 3] {
    let full = ui.max_rect();
    let gap = ui.spacing().item_spacing.x;
    let list = splitter(ui, &format!("{kind}.list"), true, SIDE_MIN, CANVAS_MIN + SIDE_MIN + 8.0, LIST_W);
    let list_rect = Rect::from_min_max(full.min, pos2((full.min.x + list.size).min(full.max.x), full.max.y));
    let rest = Rect::from_min_max(pos2((list_rect.max.x + gap).min(full.max.x), full.min.y), full.max);
    let mut child = ui.new_child(
        UiBuilder::new().id_salt((kind, "rest")).max_rect(rest).layout(Layout::left_to_right(Align::Min)),
    );
    let tail = splitter_tail(&mut child, &format!("{kind}.details"), true, CANVAS_MIN, SIDE_MIN, GRID_W);
    let preview =
        Rect::from_min_max(rest.min, pos2((rest.max.x - tail.size - gap).max(rest.min.x + 1.0), rest.max.y));
    let details = Rect::from_min_max(pos2(rest.max.x - tail.size, rest.min.y), rest.max);
    [list_rect, preview, details]
}

/// A pane: its padding, its fill, and a vertical scroll when `scroll`.
pub fn pane<R>(
    ui: &mut Ui,
    rect: Rect,
    salt: &str,
    canvas: bool,
    scroll: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    if canvas {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
    }
    let mut child = ui.new_child(
        UiBuilder::new().id_salt(salt).max_rect(rect.shrink(8.0)).layout(Layout::top_down(Align::Min)),
    );
    if scroll {
        ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(&mut child, add).inner
    } else {
        add(&mut child)
    }
}

/// What a library tab answers `library` with, and what it does for it.
pub trait LibraryDoc {
    /// Applies `search=`, then `select` (`item=`, `frame=`), and answers the kind's fields after
    /// `tab` and `active` - or the error sentence.
    fn mcp(&mut self, action: &str, args: &Args) -> Result<Map<String, Value>, String>;
    /// What a `select` must wait for before it answers (the selected texture's picture, the
    /// sound's file).
    fn settle(&self) -> Option<Settle> {
        None
    }
}

/// What stopped a tab's save, kept for whoever waits for it (`Document::save_failed`).
pub(crate) type SaveError = Arc<parking_lot::Mutex<Option<gz_editor::CmdError>>>;

/// A save runs as the job `id`: a job that fails or is stopped is the save's answer.
pub(crate) fn answer_job(editor: &Arc<Editor>, id: gz_editor::JobId, failed: &SaveError) {
    let (ed, failed) = (editor.clone(), failed.clone());
    let _task = editor.exec.spawn(async move {
        let Some(info) = ed.jobs.wait(id).await else { return };
        if info.state != gz_editor::JobState::Done {
            let why = info.error.unwrap_or_else(|| "the save was stopped".to_owned());
            *failed.lock() = Some(gz_editor::CmdError::new("failed", why));
            ed.exec.wake().ping();
        }
    });
}

/// A save found the libraries' save already running: that job's end is this save's answer too.
pub(crate) fn answer_running(editor: &Arc<Editor>, failed: &SaveError) {
    if let Some(job) = editor.jobs.running_in("libraries") {
        answer_job(editor, job.id, failed);
    }
}

/// Something a library tab is still reading for its selection.
pub type Settle = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// `rawdata\textures` and the like are written only when the author can write there.
#[must_use]
pub fn can_write(dir: &std::path::Path) -> bool {
    match std::fs::metadata(dir) {
        Ok(m) => m.is_dir() && !m.permissions().readonly(),
        // a folder that is not there yet is made by the first save
        Err(_) => {
            dir.parent().is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly()))
        }
    }
}

/// The text of a library folder of the editor (or the project).
#[must_use]
pub fn folder_text(editor: &Editor, dir: &std::path::Path) -> String {
    crate::paths::folder_text(editor.project_root().as_deref(), &gz_project::library_root(editor), dir)
}

/// The keys of a listing as `&str`, for the list.
#[must_use]
pub fn keys_of<T>(items: &[T], key: impl Fn(&T) -> &str) -> Vec<&str> {
    items.iter().map(key).collect()
}

/// The libraries' lists: a walk of each library's folder on the IO pool, kept per revision; a
/// reload or a save moves the revision. An `Editor` service.
pub struct LibraryLists {
    pub exec: gz_task::Executors,
    pub textures: gz_query::Derived<Vec<textures::TexItem>>,
    pub textures_rev: gz_base::RevCounter,
    pub sounds: gz_query::Derived<Vec<sounds::SoundItem>>,
    pub sounds_rev: gz_base::RevCounter,
}

impl LibraryLists {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<LibraryLists> {
        editor.services.get_or_init(|| LibraryLists {
            exec: editor.exec.clone(),
            textures: gz_query::Derived::new(editor.exec.clone()),
            textures_rev: gz_base::RevCounter::new(),
            sounds: gz_query::Derived::new(editor.exec.clone()),
            sounds_rev: gz_base::RevCounter::new(),
        })
    }
}
