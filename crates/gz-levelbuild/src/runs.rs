//! The programs the editor starts as processes of their own (`CLevelTool::RunGame`): started
//! with the C++ command lines, their output followed line by line, Stop ends the whole process
//! tree. The level compilers are no programs any more: they run in the editor's process
//! (`crate::compile`).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gz_task::Cancel;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// What holds the `level_run` job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// The level compilers, in process.
    Compile,
    /// The AI compiler (`compile_aimap`, `compile_spawn`).
    XrAi,
    /// The game.
    Game,
}

impl Program {
    /// The name `level_build status` answers in `running`.
    #[must_use]
    pub fn exe(self) -> &'static str {
        match self {
            Self::Compile => "gz-lc",
            Self::XrAi => "xrAI.exe",
            Self::Game => "Xr3DA.exe",
        }
    }

    /// The name the author reads in a message.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Compile => "the level compiler",
            Self::XrAi => "the AI compiler (xrAI)",
            Self::Game => "the game",
        }
    }
}

/// The command line after the executable: the given parameters.
#[must_use]
pub fn arguments(params: &str) -> Vec<String> {
    params.split_whitespace().map(str::to_owned).collect()
}

/// The C++ log line (`* Run xrAI.exe -f l01.`).
#[must_use]
pub fn command_line(program: Program, args: &[String]) -> String {
    let mut s = program.exe().to_owned();
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    s
}

/// The executable in the first folder that has it (the editor's own folder, then
/// `Bin\x64\Release` under the editor's root).
#[must_use]
pub fn find_exe(dirs: &[PathBuf], program: Program) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(program.exe())).find(|p| p.is_file())
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Exited(Option<i32>),
    Stopped,
}

/// Ends a process and every process it started (`taskkill /T /F`).
fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
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
