//! `list_commands` and `exec_command`: the level editor's numbered table over MCP and in macros.
//! They leave with the level's actions (stream L2): nothing else has a row in the table.

use std::sync::Arc;

use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, Registry, Reply, Value, json};
use gz_ui::dialog::{self, Asker};
use serde_json::Map;

use crate::cpp::{CommandVar, CppAction};
use crate::doc::DocCx;
use crate::queue::on_ui;
use crate::shell::ShellState;

fn invalid(text: impl Into<String>) -> CmdError {
    CmdError::invalid(text)
}

fn str_arg<'a>(args: &'a Args, key: &str) -> &'a str {
    args.str(key).unwrap_or("")
}

/// The keys a face of the table is bound to now, as text.
fn face_keys(shell: &ShellState, face: Option<usize>) -> String {
    face.and_then(|i| shell.binding(i)).map(|c| c.label()).unwrap_or_default()
}

async fn list_commands(editor: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    // a Run Macro slot's preset carries the file the slot runs
    let macro_files: Vec<String> =
        (0..crate::macros::SLOTS).map(|i| crate::macros::slot_file(&editor, i)).collect();
    on_ui(&editor, move |shell, _| {
        let rows: Vec<Value> = shell
            .parts
            .cpp
            .rows
            .iter()
            .map(|r| {
                let mut row = Map::new();
                row.insert("id".into(), json!(r.cmd.id));
                row.insert("name".into(), json!(r.cmd.name));
                if !r.cmd.desc.is_empty() {
                    row.insert("menu".into(), json!(r.cmd.desc));
                }
                // presets beyond the implicit one are the documented parameter combinations
                if r.cmd.subs.len() > 1 {
                    let presets: Vec<Value> = r
                        .cmd
                        .subs
                        .iter()
                        .zip(&r.faces)
                        .enumerate()
                        .map(|(i, (s, f))| {
                            let mut preset = crate::cpp::preset_json(s, &face_keys(shell, *f));
                            if r.cmd.id == crate::macros::RUN_MACRO_ID
                                && let Some(file) = macro_files.get(i)
                            {
                                preset["p1"] = json!(file);
                            }
                            preset
                        })
                        .collect();
                    row.insert("presets".into(), Value::Array(presets));
                }
                row.insert("keys".into(), json!(face_keys(shell, r.faces.first().copied().flatten())));
                Value::Object(row)
            })
            .collect();
        Ok(json!({ "commands": rows }))
    })
    .await
}

/// What a command run came to: the row, the answer, and why it refused when it did.
pub(crate) struct Ran {
    pub id: u32,
    pub name: String,
    pub result: CommandVar,
    pub refused: Option<String>,
}

/// Why a command did not run at all.
pub(crate) enum NotRun {
    Unknown,
    NoSub(String),
    Other(CmdError),
}

/// What the UI thread found and did.
enum Step {
    Done(Ran),
    Call(u32, String, &'static str, Args),
    Task(u32, String, crate::cpp::TaskFn, CommandVar, CommandVar),
}

/// Runs a row of the numbered table - the one `exec_command`, a macro's line and a face run - with
/// its sub-command (named, or picked by the parameters) and the parameters given over the
/// sub-command's own. Off the UI thread; what the row does there runs there. `origin` is who
/// asked: a caller over MCP has decided, so a question the row raises is answered, never shown.
// the row, its sub-command, both parameters and the caller: what `exec_command` carries
#[allow(clippy::too_many_arguments)]
pub(crate) async fn exec(
    editor: &Arc<Editor>,
    id: Option<i64>,
    name: String,
    sub: Option<String>,
    p1: Option<CommandVar>,
    p2: Option<CommandVar>,
    depth: usize,
    origin: Origin,
) -> Result<Ran, NotRun> {
    let step = on_ui(editor, move |shell, ctx| {
        let _caller = (origin == Origin::Mcp).then(|| dialog::answer_as(ctx, Asker::Mcp));
        let Some(at) = shell.parts.cpp.find(id, &name) else { return Ok(Err(NotRun::Unknown)) };
        let Some(row) = shell.parts.cpp.rows.get(at) else { return Ok(Err(NotRun::Unknown)) };
        // a sub-command is named by a line (`NAME.Sub(..)`) or by the name (`NAME."Sub"`)
        let named = sub.filter(|s| !s.is_empty()).or_else(|| {
            name.split_once('.')
                .map(|(_, s)| s.trim_matches('"').to_owned())
                .filter(|s| !s.is_empty() && !row.cmd.desc.eq_ignore_ascii_case(&name))
        });
        let k = match &named {
            Some(s) => match row.cmd.subs.iter().position(|x| x.desc.eq_ignore_ascii_case(s)) {
                Some(k) => k,
                None => return Ok(Err(NotRun::NoSub(s.clone()))),
            },
            None => crate::cpp::pick_sub(&row.cmd.subs, p1.as_ref(), p2.as_ref()).unwrap_or(0),
        };
        let preset = row.cmd.subs.get(k).cloned();
        let a = p1.or_else(|| preset.as_ref().map(|s| s.p1.clone())).unwrap_or_default();
        let b = p2.or_else(|| preset.as_ref().map(|s| s.p2.clone())).unwrap_or_default();
        let (row_id, row_name) = (row.cmd.id, row.cmd.name.to_string());
        let action = row.cmd.action.clone();
        let face = row.faces.get(k).copied().flatten();
        let enabled = row.cmd.enabled.clone();
        crate::cpp::log_command(&shell.editor, &row_name, &a, &b, depth);
        let done = |result: CommandVar, refused: Option<String>| {
            Ok(Ok(Step::Done(Ran { id: row_id, name: row_name.clone(), result, refused })))
        };
        if let Some(f) = enabled
            && !f(&shell.editor, &shell.active)
        {
            return done(CommandVar::Int(0), Some("the command is not available now".to_owned()));
        }
        if matches!(action, CppAction::Ui) {
            // logged above, with its depth
            shell.quiet_log = true;
            let ran = face.is_some_and(|i| shell.run_command(ctx, i));
            shell.quiet_log = false;
            let why = (!ran).then(|| "the command is not available now".to_owned());
            return done(CommandVar::Int(u32::from(ran)), why);
        }
        let outcome = crate::cpp::run_now(&mut crate::shell::cx!(shell, ctx), &action, &a, &b);
        match outcome {
            crate::cpp::Outcome::Done(v) => done(v, None),
            crate::cpp::Outcome::Refused(why) => done(CommandVar::Int(0), Some(why)),
            crate::cpp::Outcome::Call(c, args) => Ok(Ok(Step::Call(row_id, row_name, c, args))),
            crate::cpp::Outcome::Task(t, a, b) => Ok(Ok(Step::Task(row_id, row_name, t, a, b))),
        }
    })
    .await
    .map_err(NotRun::Other)??;
    Ok(match step {
        Step::Done(r) => r,
        Step::Call(id, name, command, args) => {
            let reply = editor.call(command, args, origin).await;
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                Ran { id, name, result: CommandVar::Int(1), refused: None }
            } else {
                let why =
                    reply.get("error").and_then(Value::as_str).unwrap_or("the command failed").to_owned();
                Ran { id, name, result: CommandVar::Int(0), refused: Some(why) }
            }
        }
        Step::Task(id, name, task, a, b) => match task(editor.clone(), a, b, origin).await {
            Ok(v) => Ran { id, name, result: v, refused: None },
            Err(why) => Ran { id, name, result: CommandVar::Int(0), refused: Some(why) },
        },
    })
}

/// One line of a command list: the first that is unknown or does not run is the error.
pub(crate) async fn exec_line(
    editor: &Arc<Editor>,
    line: &crate::macros::Line,
    depth: usize,
    origin: Origin,
) -> Result<(), String> {
    let sub = (!line.sub.is_empty()).then(|| line.sub.clone());
    match exec(editor, None, line.name.clone(), sub, line.p1.clone(), line.p2.clone(), depth, origin).await {
        Ok(r) if r.refused.is_none() && r.result != CommandVar::Int(0) => Ok(()),
        Ok(r) => {
            let why = r.refused.map(|w| format!(" - {w}")).unwrap_or_default();
            Err(format!("Can't execute command: '{}'{why}", line.text))
        }
        Err(NotRun::Unknown) => Err(format!("Can't find command: '{}'", line.text)),
        Err(NotRun::NoSub(s)) => Err(format!("Can't find sub-command: '{s}'")),
        Err(NotRun::Other(e)) => Err(e.message),
    }
}

/// A parameter as `exec_command` takes it: `p1s` a text, `p1i` a number (`-1` is none).
fn param(args: &Args, s: &str, i: &str) -> Option<CommandVar> {
    if let Some(t) = args.text(s) {
        return Some(CommandVar::Str(t.to_owned()));
    }
    args.i64(i).filter(|v| *v >= 0).map(|v| CommandVar::Int(u32::try_from(v).unwrap_or(u32::MAX)))
}

async fn exec_command(editor: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    // an escaped "Edit\Undo" may arrive with both slashes
    let name = str_arg(&args, "name").replace("\\\\", "\\");
    let id = args.i64("id").filter(|v| *v >= 0);
    let (p1, p2) = (param(&args, "p1s", "p1i"), param(&args, "p2s", "p2i"));
    if id.is_none() && name.is_empty() {
        return Err(invalid("unknown command - see list_commands"));
    }
    match exec(&editor, id, name, None, p1, p2, 0, origin).await {
        Ok(r) => {
            let mut out = Map::new();
            out.insert("id".into(), json!(r.id));
            out.insert("name".into(), json!(r.name));
            out.insert("result".into(), r.result.json());
            if let Some(why) = r.refused {
                out.insert("refused".into(), json!(why));
            }
            Ok(Value::Object(out))
        }
        Err(NotRun::Unknown) => Err(invalid("unknown command - see list_commands")),
        Err(NotRun::NoSub(s)) => Err(invalid(format!("Can't find sub-command: '{s}'"))),
        Err(NotRun::Other(e)) => Err(e),
    }
}

/// Registers `list_commands` and `exec_command` (no cards: the level's table goes with them).
pub(crate) fn register(reg: &mut Registry) {
    reg.add("level-table", "list_commands", list_commands);
    reg.add("level-table", "exec_command", exec_command);
}
