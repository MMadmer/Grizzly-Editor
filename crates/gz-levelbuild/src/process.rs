//! The game as a process of its own (`CLevelTool::RunGame`): started with its command line, its
//! output followed line by line, Stop ends the whole process tree. Nothing else the editor builds
//! with is a program: the level compilers run in the editor's process (`crate::compile`), and the
//! AI map, the game graph and the spawn are built there too.

use std::path::Path;
use std::process::Stdio;

use gz_task::Cancel;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// The command line after the executable: the given parameters.
#[must_use]
pub fn arguments(params: &str) -> Vec<String> {
    params.split_whitespace().map(str::to_owned).collect()
}

/// The C++ log line's command (`* Run xrEngine.exe -fsltx fsgame.ltx ...`): the executable's
/// file name and the arguments.
#[must_use]
pub fn command_line(exe: &Path, args: &[String]) -> String {
    let mut s = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    s
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Exited(Option<i32>),
    Stopped,
}

/// A console program (`taskkill`; the tests' `cmd`) as a command that opens no console window: the
/// editor is a window program, so one would flash up for it otherwise. The game's engine is a
/// window program and is started as it is.
#[must_use]
pub fn console(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Ends a process and every process it started (`taskkill /T /F`).
fn kill_tree(pid: u32) {
    let _ = console("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Runs `exe` in `cwd` until it ends or `cancel` asks to stop. Every line it prints goes to
/// `on_line`. A program that does not start is a sentence for the author.
pub async fn run(
    exe: &Path,
    args: &[String],
    cwd: &Path,
    cancel: &Cancel,
    on_line: impl Fn(&str) + Send + Sync,
) -> Result<Ended, String> {
    let mut child = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{} could not be started: {e}", exe.display()))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut out_lines = stdout.map(|s| BufReader::new(s).lines());
    let mut err_lines = stderr.map(|s| BufReader::new(s).lines());
    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                if let Some(pid) = pid {
                    tokio::task::spawn_blocking(move || kill_tree(pid)).await.ok();
                }
                let _ = child.kill().await;
                return Ok(Ended::Stopped);
            }
            line = async {
                match out_lines.as_mut() {
                    Some(l) => l.next_line().await,
                    None => std::future::pending().await,
                }
            } => match line {
                Ok(Some(l)) => on_line(&l),
                _ => out_lines = None,
            },
            line = async {
                match err_lines.as_mut() {
                    Some(l) => l.next_line().await,
                    None => std::future::pending().await,
                }
            } => match line {
                Ok(Some(l)) => on_line(&l),
                _ => err_lines = None,
            },
            status = child.wait(), if out_lines.is_none() && err_lines.is_none() => {
                return Ok(Ended::Exited(status.ok().and_then(|s| s.code())));
            }
        }
    }
}
