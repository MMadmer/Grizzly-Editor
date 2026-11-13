//! Commandlets: the editor run from a shell for one piece of work.
//!
//! `Grizzly-Cmd.exe run <command> [name=value ...]`, `script <file>`, `help [about]` and their
//! short words (`check`, `build`, ...). A commandlet is the editor without a window and without
//! a control endpoint: it opens the project, waits until the project is ready, runs its
//! commands the way a caller over MCP would (nothing ever asks), waits for every job they
//! started, prints the answers and ends. Exit code 0: everything worked; 1: a command failed;
//! 2: the run could not start.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use gz_editor::{Args, Editor, Origin};
use gz_task::Runtime;
use serde_json::{Map, Value, json};

use crate::cli::{Cli, Verb};

/// The exit code of a run that could not start: a wrong command line, a project that did not
/// open, a script that cannot be read.
const NOT_RUN: u8 = 2;

fn out(text: &str) -> bool {
    writeln!(std::io::stdout().lock(), "{text}").is_ok()
}

fn err(text: &str) {
    // nobody listening is no reason to fail another way
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}

/// Whether a folder is a project: it holds a project file or a module manifest.
fn is_project(dir: &Path) -> bool {
    gz_project::list::has_manifest(dir)
}

/// The project a commandlet works on: `-project`, else the current folder when it is one.
fn project_of(cli: &Cli) -> Option<PathBuf> {
    cli.project.clone().or_else(|| std::env::current_dir().ok().filter(|d| is_project(d)))
}

/// One command of a script or of the command line.
struct Step {
    command: String,
    args: Args,
}

/// The parameters as a caller over JSON would send them. Text stays text: a command with a
/// card reads it for what it says. A value that is plainly JSON (`{...}`, `[...]`) is taken as
/// such for a command that reads its arguments itself.
fn args_of(editor: &Editor, command: &str, pairs: &[(String, String)]) -> Args {
    let reads_itself = editor.registry().card(command).is_none();
    let mut args = Map::new();
    for (name, value) in pairs {
        let text = value.trim();
        let parsed = (reads_itself && (text.starts_with('{') || text.starts_with('[')))
            .then(|| serde_json::from_str::<Value>(text).ok())
            .flatten();
        args.insert(name.clone(), parsed.unwrap_or_else(|| Value::String(value.clone())));
    }
    args
}

/// Whether a command works on the open level: then the project's scene comes up with the
/// project. A command without a card may be one of those, so it gets the scene too.
fn wants_scene(editor: &Editor, command: &str) -> bool {
    match editor.registry().card(command) {
        Some((spec, _)) => matches!(spec.area(), "scene" | "level" | "viewport" | "shot"),
        None => true,
    }
}

/// Splits a script line into words; double quotes keep spaces, `\"` is a quote inside them.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted && chars.peek() == Some(&'"') => {
                word.push('"');
                chars.next();
            }
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => word.push(c),
        }
    }
    if any || !word.is_empty() {
        out.push(word);
    }
    out
}

/// A script: a command per line as `command name=value ...` or as a JSON request; `#` starts a
/// comment line.
fn script_steps(editor: &Editor, text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |why: String| format!("line {}: {why}", n + 1);
        if line.starts_with('{') {
            let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(line) else {
                return Err(at("a line that starts with { is a JSON request".to_owned()));
            };
            let Some(Value::String(command)) = map.remove("cmd") else {
                return Err(at("missing 'cmd'".to_owned()));
            };
            let args = match map.remove("args") {
                Some(Value::Object(args)) => args,
                _ => map,
            };
            steps.push(Step { command, args });
            continue;
        }
        let mut parts = words(line).into_iter();
        let Some(command) = parts.next() else { continue };
        let mut pairs = Vec::new();
        for word in parts {
            match word.split_once('=') {
                Some((name, value)) if !name.is_empty() => pairs.push((name.to_owned(), value.to_owned())),
                _ => return Err(at(format!("'{word}': write a parameter as name=value"))),
            }
        }
        let args = args_of(editor, &command, &pairs);
        steps.push(Step { command, args });
    }
    Ok(steps)
}

/// Prints an answer the way a shell reader wants it: a text as the text, anything else as
/// indented JSON; a failure as its sentence on stderr, with the usage when it has one.
/// `as_json` prints the whole answer as one line instead. Answers whether it worked.
fn print(reply: &Value, as_json: bool) -> bool {
    let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
    if as_json {
        out(&reply.to_string());
        return ok;
    }
    let Some(map) = reply.as_object() else {
        out(&reply.to_string());
        return ok;
    };
    if !ok {
        let sentence = map.get("error").and_then(Value::as_str).unwrap_or("the command failed");
        err(&format!("error: {sentence}"));
        if let Some(usage) = map.get("usage").and_then(Value::as_str) {
            err(usage.trim_end());
        }
    }
    let rest: Map<String, Value> = map
        .iter()
        .filter(|(k, _)| k.as_str() != "ok" && (ok || !matches!(k.as_str(), "error" | "usage" | "code")))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    match rest.get("text").and_then(Value::as_str) {
        Some(text) if rest.len() == 1 => {
            out(text.trim_end());
        }
        _ if rest.is_empty() => {}
        _ => {
            let pretty = serde_json::to_string_pretty(&Value::Object(rest)).unwrap_or_default();
            if ok {
                out(&pretty);
            } else {
                err(&pretty);
            }
        }
    }
    ok
}

/// Writes what the running jobs do to stderr, a line per change, until the task is dropped.
async fn follow_jobs(editor: Arc<Editor>) {
    let mut last = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let running: Vec<_> = editor.jobs.snapshot().into_iter().filter(gz_task::JobInfo::running).collect();
        let Some(job) = running.first() else { continue };
        let percent =
            job.fraction.map(|f| format!(" {:.0}%", (f * 100.0).clamp(0.0, 100.0))).unwrap_or_default();
        let line = if job.stage.is_empty() {
            format!("[{}]{percent}", job.title)
        } else {
            format!("[{}] {}{percent}", job.title, job.stage)
        };
        if line != last {
            err(&line);
            last = line;
        }
    }
}

/// Waits until no job runs: a project that just opened loads its scene in one, and a command
/// may leave work behind.
async fn settle(editor: &Arc<Editor>) {
    editor.call("job.wait", Args::new(), Origin::Mcp).await;
}

fn usage(why: &str) -> ExitCode {
    err(&format!("usage: Grizzly-Cmd.exe {why}"));
    err("  run <command> [name=value ...]   one command; help lists them");
    err("  script <file>                    a command per line (- reads the standard input)");
    err("  help [area | command | topic]");
    err("  check [path] | reference <kind> | build | package | update");
    err("  serve                            the editor without a window, on its control endpoint");
    err("options: -project <folder or project file>  -json  -quiet  -keep_going  -home <folder>");
    ExitCode::from(NOT_RUN)
}

/// Runs the commandlet `cli` names and answers the process's exit code. The caller closes the
/// project afterwards, as for every run.
pub fn run(runtime: &Runtime, editor: &Arc<Editor>, cli: &Cli) -> ExitCode {
    if let Some(why) = &cli.usage {
        return usage(why);
    }
    let Some(verb) = &cli.verb else { return usage("<verb>") };
    // help needs no project; everything else works on one when there is one
    let steps = match verb {
        Verb::Help { about } => {
            let mut args = Args::new();
            if let Some(about) = about {
                args.insert("about".to_owned(), json!(about));
            }
            vec![Step { command: "help".to_owned(), args }]
        }
        Verb::Run { command, args } => {
            vec![Step { command: command.clone(), args: args_of(editor, command, args) }]
        }
        Verb::Script { file } => {
            let text = if file.as_os_str() == "-" {
                let mut text = String::new();
                std::io::stdin().lock().read_to_string(&mut text).map(|_| text)
            } else {
                std::fs::read_to_string(file)
            };
            let text = match text {
                Ok(text) => text,
                Err(e) => {
                    err(&format!("error: the script {} cannot be read: {e}", file.display()));
                    return ExitCode::from(NOT_RUN);
                }
            };
            match script_steps(editor, &text) {
                Ok(steps) => steps,
                Err(why) => {
                    err(&format!("error: {}: {why}", file.display()));
                    return ExitCode::from(NOT_RUN);
                }
            }
        }
        Verb::Serve => return usage("serve"),
    };

    let progress = (!cli.quiet).then(|| runtime.exec().spawn(follow_jobs(editor.clone())));
    let code = runtime.block_on(async {
        if !matches!(verb, Verb::Help { .. })
            && let Some(project) = project_of(cli)
        {
            let mut args = Args::new();
            args.insert("path".to_owned(), json!(project.display().to_string()));
            // work on files does not wait for a level to load
            args.insert("scene".to_owned(), json!(steps.iter().any(|s| wants_scene(editor, &s.command))));
            let reply = editor.call("project.open", args, Origin::Mcp).await;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let why = reply.get("error").and_then(Value::as_str).unwrap_or("unknown error");
                err(&format!("error: the project {} did not open: {why}", project.display()));
                return NOT_RUN;
            }
            settle(editor).await;
        }
        let mut failed = false;
        for step in steps {
            let reply = editor.call(&step.command, step.args, Origin::Mcp).await;
            // what the command left running is part of it
            settle(editor).await;
            if !print(&reply, cli.json) {
                failed = true;
                if !cli.keep_going {
                    break;
                }
            }
        }
        u8::from(failed)
    });
    if let Some(task) = progress {
        task.abort();
    }
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_line_splits_into_words_with_quotes() {
        assert_eq!(
            words(r#"scene.place kind=object name="a b" note="say \"hi\"""#),
            ["scene.place", "kind=object", "name=a b", r#"note=say "hi""#]
        );
        assert_eq!(words("  one   two  "), ["one", "two"]);
        assert_eq!(words(r#"x="""#), ["x="]);
    }
}
