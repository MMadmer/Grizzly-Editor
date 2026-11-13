//! The project file's association for the current user: a double click on `<Name>.grizzly` opens
//! the project in this editor. `HKCU\Software\Classes`: the extension `.grizzly`, the type
//! `GrizzlyEditor.Project` with the editor's icon and `"<exe>" "%1"`. Written by the author's own
//! editor at start when it is missing or names a program that no longer exists (Preferences >
//! General > Open .grizzly Files with This Editor), and on request (Register Now,
//! `project.associate now=true`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::Editor;

use crate::games::Machine;

/// The type the extension names.
pub const PROG_ID: &str = "GrizzlyEditor.Project";
/// Where the current user's file types live.
pub const CLASSES: &str = r"Software\Classes";

/// What the association is on this machine now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct State {
    /// The program the extension opens (`None`: no association).
    pub program: Option<String>,
    /// It is this editor.
    pub ours: bool,
    /// The program it names exists.
    pub exists: bool,
}

/// The association under `base` (the real one: [`CLASSES`]) for `program`.
#[cfg(windows)]
fn association<'a>(base: &'a str, program: &'a Path) -> gz_win::Association<'a> {
    gz_win::Association {
        base,
        extension: crate::project::EXTENSION,
        prog_id: PROG_ID,
        description: "Grizzly Editor Project",
        program,
    }
}

/// What the association under `base` is, seen from `program`.
#[must_use]
pub fn state(base: &str, program: &Path) -> State {
    #[cfg(windows)]
    {
        let now = association(base, program).registered_program();
        let exists = now.as_deref().is_some_and(|p| Path::new(p).is_file());
        let ours = now.as_deref().is_some_and(|p| crate::paths::same_path(p, &program.display().to_string()));
        State { program: now, ours, exists }
    }
    #[cfg(not(windows))]
    {
        let _ = (base, program);
        State { program: None, ours: false, exists: false }
    }
}

/// Writes the association under `base` for `program`.
pub fn register(base: &str, program: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    return association(base, program).register();
    #[cfg(not(windows))]
    {
        let _ = (base, program);
        Ok(())
    }
}

/// Whether the start should write it: nothing is there, or the program it names is gone.
#[must_use]
pub fn wanted(now: &State) -> bool {
    now.program.is_none() || !now.exists
}

/// This editor's program.
#[must_use]
pub fn this_program() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// At the start of the author's own editor: the association written when it is missing or
/// stale and the preference is on. Off the UI thread; the outcome goes to the log.
pub fn at_start(editor: &Arc<Editor>) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let machine = Machine::of(&ed);
        let on = machine.load().await.map(|m| m.associate()).unwrap_or(true);
        let Some(program) = this_program() else { return };
        if !on {
            return;
        }
        let log = ed.log.clone();
        ed.exec
            .io(move || {
                let now = state(CLASSES, &program);
                if !wanted(&now) {
                    return;
                }
                match register(CLASSES, &program) {
                    Ok(()) => log.info(format!(".grizzly files open with {}", program.display())),
                    Err(e) => log.warn(format!("the .grizzly file association could not be written: {e}")),
                }
            })
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Under a key of the test's own: never the real `Software\Classes`.
    #[cfg(windows)]
    #[test]
    fn the_start_writes_what_is_missing_or_stale_and_leaves_a_good_one() {
        let top = format!(r"Software\GrizzlyEditorTest-assoc-{}", std::process::id());
        let base = format!(r"{top}\Classes");
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("grizzly-test-program");
        std::fs::write(&program, "x").unwrap();
        let none = state(&base, &program);
        assert_eq!(none.program, None);
        assert!(wanted(&none));
        register(&base, &program).unwrap();
        let ours = state(&base, &program);
        assert!(ours.ours && ours.exists && !wanted(&ours), "{ours:?}");
        // another program that still exists is the author's choice; one that is gone is stale
        let other = dir.path().join("another-program");
        std::fs::write(&other, "x").unwrap();
        register(&base, &other).unwrap();
        assert!(!wanted(&state(&base, &program)));
        std::fs::remove_file(&other).unwrap();
        assert!(wanted(&state(&base, &program)));
        gz_win::delete_user_key(&top).unwrap();
    }
}
