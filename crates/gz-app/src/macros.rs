//! Macros (`UI_MainCommand.cpp` `CommandExecuteCommandList`, `CommandRunMacro`,
//! `CommandAssignMacro`, `CommandLogCommands`): a text of commands, one a line, run through the
//! same path as `exec_command`, off the UI thread; the first refusal stops it and names the line.
//!
//! The file format is the C++ editor's: `COMMAND_NAME[.Sub](p1, p2)` a line, `;` or `//` start a
//! comment line, a parameter is a number or a quoted text, a missing one keeps the sub-command's.
//! The eight slots of Run Macro keep their file in the settings under the C++ key
//! (`[macros] COMMAND_RUN_MACRO."Slot #1"`); a file is looked for as given, then in the editor's
//! `import` folder (`$import$`), and one picked there is kept relative to it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gz_editor::{Editor, Origin};
use gz_ui::Icon;

use crate::cpp::{CommandVar, CppAction, CppCommand, CppSub};
use crate::settings::EditorSettings;

/// How many slots Run Macro has.
pub const SLOTS: usize = 8;
/// How deep a macro may run macros.
const MAX_DEPTH: usize = 8;

/// The C++ id of `COMMAND_RUN_MACRO`.
pub(crate) const RUN_MACRO_ID: u32 = 52;

/// The settings key of a slot (`COMMAND_RUN_MACRO."Slot #1"`, 0-based `slot`).
#[must_use]
pub fn slot_key(slot: usize) -> String {
    format!("COMMAND_RUN_MACRO.\"Slot #{}\"", slot + 1)
}

/// One line of a command list, read as `ExecuteCommandList` reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// As written, for the message that names it.
    pub text: String,
    pub name: String,
    /// The sub-command's name; empty for none.
    pub sub: String,
    pub p1: Option<CommandVar>,
    pub p2: Option<CommandVar>,
}

/// `_GetItem(src, index, dst, sep)`: the trimmed `index`-th part.
fn item(src: &str, index: usize, sep: char) -> &str {
    src.split(sep).nth(index).unwrap_or("").trim()
}

/// The lines of a command list that are commands (comments and empty lines left out).
#[must_use]
pub fn parse(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with("//") {
            continue;
        }
        let cmd = item(line, 0, '(');
        let params = item(item(line, 1, '('), 0, ')');
        let (sp1, sp2) = (item(params, 0, ','), item(params, 1, ','));
        out.push(Line {
            text: cmd.to_owned(),
            name: item(cmd, 0, '.').to_owned(),
            // a sub-command may be written quoted, as the shortcut keys name it
            sub: item(cmd, 1, '.').trim_matches('"').to_owned(),
            p1: CommandVar::parse_param(sp1),
            p2: CommandVar::parse_param(sp2),
        });
    }
    out
}

/// The file a macro names: as given, else under the editor's `import` folder.
fn macro_file(editor: &Editor, name: &str) -> Option<PathBuf> {
    let given = PathBuf::from(name);
    if given.is_file() {
        return Some(given);
    }
    let under = import_dir(editor).join(name);
    under.is_file().then_some(under)
}

fn import_dir(editor: &Editor) -> PathBuf {
    editor.paths.root.join("import")
}

/// A file picked for a slot: relative to `import` when it lies there (`CommandAssignMacro`).
fn slot_name(editor: &Editor, file: &str) -> String {
    let dir = import_dir(editor);
    let dir_text = format!("{}\\", dir.display());
    match file.get(..dir_text.len()) {
        Some(head) if head.eq_ignore_ascii_case(&dir_text) => {
            file.get(dir_text.len()..).unwrap_or("").to_owned()
        }
        _ => file.to_owned(),
    }
}

/// How deep the command lists that run now are nested (`s_CommandExecutionDepth`).
#[derive(Default)]
struct Depth(AtomicUsize);

/// Holds one level of [`Depth`] while a list runs.
struct Level(Arc<Depth>);

impl Drop for Level {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Runs a command list: each line through `exec_command`'s path, in order; the first that is
/// unknown or refused stops the list and is the error. `origin` is who asked for the list: its
/// lines answer their questions the same way.
pub async fn run_list(editor: &Arc<Editor>, text: &str, origin: Origin) -> Result<(), String> {
    let depth = editor.services.get_or_init(Depth::default);
    let level = depth.0.fetch_add(1, Ordering::AcqRel) + 1;
    let _held = Level(depth);
    if level > MAX_DEPTH {
        return Err("Macros run each other too deep.".to_owned());
    }
    for line in parse(text) {
        crate::cmds::exec_line(editor, &line, level, origin).await?;
    }
    Ok(())
}

/// Reads a macro's file and runs it (`CommandRunMacro` with a file name).
pub async fn run_file(editor: &Arc<Editor>, name: &str, origin: Origin) -> Result<(), String> {
    let Some(file) = macro_file(editor, name) else {
        return Err(format!("Can't open the macro '{name}'."));
    };
    let text = editor
        .exec
        .io(move || std::fs::read(&file).map(|b| gz_base::text::decode_guess(&b).into_owned()))
        .await
        .map_err(|e| format!("Can't read the macro '{name}': {e}"))?;
    run_list(editor, &text, origin).await
}

/// The file of a slot (0-based); empty when none is assigned.
#[must_use]
pub fn slot_file(editor: &Editor, slot: usize) -> String {
    EditorSettings::of(editor).macro_file(&slot_key(slot))
}

/// Keeps a slot's file in the settings.
pub fn assign(editor: &Editor, slot: usize, file: &str) {
    let settings = EditorSettings::of(editor);
    settings.set_macro_file(&slot_key(slot), &slot_name(editor, file));
    settings.save_soon(editor);
}

/// The system's open dialog (the kit's, on a thread of its own), in `import`; the file picked,
/// or `None` when cancelled. A caller who cannot answer it gets "name the file".
async fn pick_file(editor: &Arc<Editor>, origin: Origin) -> Result<Option<String>, String> {
    let asker = match origin {
        Origin::Mcp => gz_ui::Asker::Mcp,
        Origin::Ui if !editor.flags.unattended => gz_ui::Asker::Ui,
        _ => gz_ui::Asker::Unattended,
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = gz_ui::file_dialog::FileRequest::open("Choose the macro").in_folder(import_dir(editor));
    let picked = gz_ui::file_dialog::pick(None, asker, request, move |p| {
        let _ = tx.send(p.display().to_string());
    });
    if let Err(line) = picked {
        editor.log.plain(line);
        return Err(gz_ui::file_dialog::NAME_THE_FILE.to_owned());
    }
    Ok(rx.await.ok())
}

fn slot_of(p: &CommandVar) -> Result<usize, String> {
    let slot = p.int().map_or(SLOTS, |v| v as usize);
    if slot >= SLOTS {
        return Err(format!("the slot is 0..{} (Slot #1 is 0)", SLOTS - 1));
    }
    Ok(slot)
}

/// The macro rows of the C++ table.
pub(crate) fn rows() -> Vec<CppCommand> {
    let execute = CppAction::task(|ed: Arc<Editor>, p1: CommandVar, _p2, origin| async move {
        let text = p1.text().ok_or_else(|| "p1s is the command list".to_owned())?.to_owned();
        run_list(&ed, &text, origin).await.map(|()| CommandVar::Int(1))
    });
    let log = CppAction::shell(|cx, p1, _p2| {
        // oracle: `bAllowLogCommands = !_p1`: a 0 turns logging on
        crate::cpp::CommandLog::of(cx.editor).set(!p1.truthy());
        Ok(CommandVar::Int(1))
    });
    let run = CppAction::task(|ed: Arc<Editor>, p1: CommandVar, _p2, origin| async move {
        let name = match &p1 {
            CommandVar::Str(s) => s.clone(),
            CommandVar::Int(_) => {
                let slot = slot_of(&p1)?;
                let file = slot_file(&ed, slot);
                if file.is_empty() {
                    return Err(format!("Run Macro Slot #{} has no file: Assign Macro picks one.", slot + 1));
                }
                file
            }
        };
        run_file(&ed, &name, origin).await.map(|()| CommandVar::Int(1))
    });
    let assign_task = CppAction::task(|ed: Arc<Editor>, p1: CommandVar, p2: CommandVar, origin| async move {
        let slot = slot_of(&p1)?;
        let file = match p2 {
            CommandVar::Str(s) => s,
            CommandVar::Int(_) => match pick_file(&ed, origin).await? {
                Some(f) => f,
                None => return Ok(CommandVar::Int(0)),
            },
        };
        assign(&ed, slot, &file);
        Ok(CommandVar::Int(1))
    });
    let slots = (0..SLOTS)
        .map(|i| {
            CppSub::new(&format!("Slot #{}", i + 1), CommandVar::Int(i as u32), CommandVar::Int(0))
                .keys(&format!("Alt+{}", i + 1))
                .label(&format!("Run Macro Slot #{}", i + 1))
        })
        .collect();
    vec![
        CppCommand::new(50, "COMMAND_EXECUTE_COMMAND_LIST", "", execute).editable(false).face(
            "Macros",
            "Execute Command List",
            Icon::Terminal,
        ),
        CppCommand::new(51, "COMMAND_LOG_COMMANDS", "", log).editable(false).face(
            "Macros",
            "Log Commands",
            Icon::Log,
        ),
        CppCommand::new(RUN_MACRO_ID, "COMMAND_RUN_MACRO", "Run Macro", run).subs(slots).face(
            "Macros",
            "Run Macro",
            Icon::Play,
        ),
        CppCommand::new(53, "COMMAND_ASSIGN_MACRO", "", assign_task).editable(false).face(
            "Macros",
            "Assign Macro",
            Icon::Terminal,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_like_execute_command_list() {
        let text = "; a comment\n// another\n\nCOMMAND_ZOOM_EXTENTS.Selected()\nCOMMAND_RUN_MACRO(\"b.txt\", 2)\n  COMMAND_UNDO  \nCOMMAND_GIZMO_MODE.\"Move\"(0,0)\n";
        let lines = parse(text);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].name, "COMMAND_ZOOM_EXTENTS");
        assert_eq!(lines[0].sub, "Selected");
        assert_eq!(lines[0].p1, None);
        assert_eq!(lines[1].p1, Some(CommandVar::Str("b.txt".into())));
        assert_eq!(lines[1].p2, Some(CommandVar::Int(2)));
        assert_eq!(lines[2].name, "COMMAND_UNDO");
        assert_eq!(lines[3].sub, "Move");
    }

    #[test]
    fn slot_keys_are_the_cpp_ones() {
        assert_eq!(slot_key(0), "COMMAND_RUN_MACRO.\"Slot #1\"");
        assert_eq!(slot_key(7), "COMMAND_RUN_MACRO.\"Slot #8\"");
    }
}
