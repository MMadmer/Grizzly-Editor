//! What the shell needs to know of the open project, without naming the area that owns it: its
//! folder, its name and whether its game is linked. The project area provides it through
//! [`set_project_probe`]; without one the shell takes the folder's name and assumes a linked game.

use gz_editor::{Editor, Value, json};

/// The open project as the frame shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInfo {
    pub active: bool,
    /// The folder, as the wire writes it.
    pub folder: String,
    /// What the caption and the OS title show.
    pub name: String,
    /// Its game is linked: without one the Start page stays up (with the Link Game card).
    pub game_linked: bool,
}

type ProbeFn = dyn Fn(&Editor) -> ProjectInfo + Send + Sync;

/// The probe an area installs; an `Editor` service.
pub struct ProjectProbe(Box<ProbeFn>);

/// Tells the shell how to read the open project. Called once, by `main`, before the window opens.
pub fn set_project_probe(editor: &Editor, probe: impl Fn(&Editor) -> ProjectInfo + Send + Sync + 'static) {
    editor.services.insert(ProjectProbe(Box::new(probe)));
}

/// The open project now.
#[must_use]
pub fn project_info(editor: &Editor) -> ProjectInfo {
    if let Some(probe) = editor.services.get::<ProjectProbe>() {
        return (probe.0)(editor);
    }
    match editor.project_root() {
        Some(root) => ProjectInfo {
            active: true,
            folder: root.display().to_string().replace('\\', "/"),
            name: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            game_linked: true,
        },
        None => ProjectInfo::default(),
    }
}

/// `project_active`, `project`, `project_name`: the head of the `state` answer.
#[must_use]
pub fn state_fields(info: &ProjectInfo) -> Value {
    json!({ "project_active": info.active, "project": info.folder, "project_name": info.name })
}
