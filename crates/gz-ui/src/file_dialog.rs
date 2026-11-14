//! The system's file dialog, the one way the editor opens it.
//!
//! A window of the operating system cannot be answered by an MCP caller or under `-nodlg`, so
//! for those askers nothing opens: [`pick`] returns the line to log and the caller refuses with
//! [`NAME_THE_FILE`]. For a person the dialog runs on a thread of its own - made by an IO pool
//! thread (`Executors::thread`), never by the frame of the click - and `then` gets the path
//! picked (it is dropped unrun when the dialog is cancelled).

use std::path::PathBuf;

use egui::Context;
use gz_base::Finding;
use gz_task::Executors;

use crate::dialog::Asker;

/// What a command answers when it wanted a file and no dialog may open for its caller.
pub const NAME_THE_FILE: &str = "name the file: no file dialog opens for this caller";

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// An existing file.
    Open,
    /// A file to write, with the name proposed.
    Save(String),
    /// A folder.
    Folder,
}

/// One request: the title, the filters, the folder it opens in and what it picks.
#[derive(Clone, Debug)]
pub struct FileRequest {
    /// The editor's own words: the dialog shows them in the window's language.
    pub title: crate::Tx<'static>,
    pub filters: Vec<(crate::Tx<'static>, Vec<String>)>,
    pub folder: Option<PathBuf>,
    pub mode: Mode,
    /// Create the folder first when it does not exist.
    pub create_folder: bool,
}

impl FileRequest {
    #[must_use]
    pub fn open<'a>(title: impl Into<crate::Tx<'a>>) -> Self {
        Self {
            title: title.into().into_owned(),
            filters: Vec::new(),
            folder: None,
            mode: Mode::Open,
            create_folder: false,
        }
    }

    #[must_use]
    pub fn save<'a>(title: impl Into<crate::Tx<'a>>, name: &str) -> Self {
        Self { mode: Mode::Save(name.to_owned()), ..Self::open(title) }
    }

    #[must_use]
    pub fn folder<'a>(title: impl Into<crate::Tx<'a>>) -> Self {
        Self { mode: Mode::Folder, ..Self::open(title) }
    }

    #[must_use]
    pub fn filter<'a>(mut self, name: impl Into<crate::Tx<'a>>, extensions: &[&str]) -> Self {
        self.filters.push((name.into().into_owned(), extensions.iter().map(|e| (*e).to_owned()).collect()));
        self
    }

    /// Opens in `dir` (when it is a folder, or `create_folder` makes it one).
    #[must_use]
    pub fn in_folder(mut self, dir: impl Into<PathBuf>) -> Self {
        self.folder = Some(dir.into());
        self
    }

    #[must_use]
    pub fn creating_folder(mut self) -> Self {
        self.create_folder = true;
        self
    }
}

/// What is logged when nothing opens.
#[must_use]
pub fn suppressed_line(title: &str) -> Finding {
    Finding::warning(format!("[dlg suppressed] {title}"))
}

/// Whether a dialog would open for `asker` on `ctx` (a caller's scope held there wins).
#[must_use]
pub fn may_open(ctx: Option<&Context>, asker: Asker) -> bool {
    let who = ctx.map_or(asker, |c| crate::dialog::who(c, asker));
    who == Asker::Ui
}

/// Opens the system's dialog for a person, on a thread of its own that `exec` makes (a call from
/// a frame costs a message to a pool, not a system call that a virus scanner may hold for
/// seconds); `then` runs there with the path picked. For any other asker (or the caller's scope
/// on `ctx`) nothing opens: the error is the finding to log, and the caller refuses with
/// [`NAME_THE_FILE`].
pub fn pick(
    exec: &Executors,
    ctx: Option<&Context>,
    asker: Asker,
    request: FileRequest,
    then: impl FnOnce(PathBuf) + Send + 'static,
) -> Result<(), Finding> {
    if !may_open(ctx, asker) {
        return Err(suppressed_line(&request.title.en()));
    }
    // the dialog's words in the window's language, worked out before its thread starts
    let title = ctx.map_or_else(|| request.title.en().into_owned(), |c| request.title.text(c));
    let filters: Vec<(String, Vec<String>)> = request
        .filters
        .iter()
        .map(|(n, e)| (ctx.map_or_else(|| n.en().into_owned(), |c| n.text(c)), e.clone()))
        .collect();
    let _started = exec.thread("gz-ui.file_dialog", move || {
        if let Some(picked) = run(&request, &title, &filters) {
            then(picked);
        }
    });
    Ok(())
}

fn run(request: &FileRequest, title: &str, filters: &[(String, Vec<String>)]) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title);
    for (name, ext) in filters {
        d = d.add_filter(name, ext);
    }
    if let Some(dir) = &request.folder {
        if request.create_folder {
            let _ = std::fs::create_dir_all(dir);
        }
        if dir.is_dir() {
            d = d.set_directory(dir);
        }
    }
    match &request.mode {
        Mode::Open => d.pick_file(),
        Mode::Save(name) => d.set_file_name(name).save_file(),
        Mode::Folder => d.pick_folder(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::answer_as;
    use gz_task::{ExecOptions, Runtime};

    #[test]
    fn nothing_opens_for_a_caller_who_cannot_answer() {
        let rt = Runtime::start(ExecOptions::default()).expect("a runtime");
        let exec = rt.exec();
        let line =
            pick(&exec, None, Asker::Unattended, FileRequest::open("Open Scene"), |_| panic!("no path"));
        assert_eq!(line, Err(Finding::warning("[dlg suppressed] Open Scene")));
        let line = pick(&exec, None, Asker::Mcp, FileRequest::save("Save Scene As", "a.level"), |_| {
            panic!("no path")
        });
        assert_eq!(line, Err(Finding::warning("[dlg suppressed] Save Scene As")));
    }

    #[test]
    fn a_held_scope_wins_over_the_asker_named() {
        let ctx = Context::default();
        assert!(may_open(Some(&ctx), Asker::Ui));
        let rt = Runtime::start(ExecOptions::default()).expect("a runtime");
        let exec = rt.exec();
        let scope = answer_as(&ctx, Asker::Mcp);
        assert!(!may_open(Some(&ctx), Asker::Ui));
        let line =
            pick(&exec, Some(&ctx), Asker::Ui, FileRequest::folder("Link Game"), |_| panic!("no path"));
        assert_eq!(line, Err(Finding::warning("[dlg suppressed] Link Game")));
        drop(scope);
        assert!(may_open(Some(&ctx), Asker::Ui));
    }
}
