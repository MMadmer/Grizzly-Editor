//! The compilers and the game as processes of their own (`CLevelTool::RunXrLC`, `RunXrDO`,
//! `RunGame`): started with the C++ command lines, their output followed line by line, Stop
//! ends the whole process tree.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gz_task::Cancel;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// What a run starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// The level compiler: geometry and light.
    XrLc,
    /// The detail objects' light.
    XrDo,
    /// The AI compiler (`compile_aimap`, `compile_spawn`).
    XrAi,
    /// The game.
    Game,
}

impl Program {
    #[must_use]
    pub fn exe(self) -> &'static str {
        match self {
            Self::XrLc => "XrLC.exe",
            Self::XrDo => "xrDO_light.exe",
            Self::XrAi => "xrAI.exe",
            Self::Game => "Xr3DA.exe",
        }
    }

    /// The name the author reads in a message.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::XrLc => "the level compiler (XrLC)",
            Self::XrDo => "the detail light compiler (xrDO_Light)",
            Self::XrAi => "the AI compiler (xrAI)",
            Self::Game => "the game",
        }
    }
}

/// The command line after the executable: `-f <level>` and then the given parameters for the
/// level compilers, the given parameters for xrAI and the game.
#[must_use]
pub fn arguments(program: Program, level: &str, params: &str) -> Vec<String> {
    let given = params.split_whitespace().map(str::to_owned);
    match program {
        Program::XrLc | Program::XrDo => {
            ["-f".to_owned(), level.to_owned()].into_iter().chain(given).collect()
        }
        Program::XrAi | Program::Game => given.collect(),
    }
}

/// The C++ log line (`* Run XrLC.exe -f l01.`).
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

/// The compilers read the `fsgame.ltx` three folders above their executable, whose levels folder
/// is the editor's, while a project's level is built into the project. So they run from a stage
/// of the project's (`temp\compilers`): copies of the folder's executables and libraries in
/// `bin\x64\Release`, beside an `fsgame.ltx` whose roots name the editor's data and, for the
/// levels and the spawns, the project's `gamedata` (the C++ editor's own `EditorProject` mounts).
/// Answers the stage's folder of executables. Blocking.
pub fn stage_compilers(
    bin: &Path,
    editor_root: &Path,
    project: &Path,
    stage: &Path,
) -> Result<PathBuf, String> {
    let out = stage.join("bin").join("x64").join("Release");
    std::fs::create_dir_all(&out).map_err(|e| format!("{} could not be made: {e}", out.display()))?;
    let entries = std::fs::read_dir(bin).map_err(|e| format!("{} could not be read: {e}", bin.display()))?;
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
        if e.file_type().is_ok_and(|t| t.is_file()) && matches!(ext.as_deref(), Some("exe" | "dll")) {
            let to = out.join(e.file_name());
            let same = matches!((std::fs::metadata(&p), std::fs::metadata(&to)),
                (Ok(a), Ok(b)) if a.len() == b.len() && a.modified().ok() == b.modified().ok());
            if !same {
                std::fs::copy(&p, &to)
                    .map_err(|err| format!("{} could not be copied: {err}", p.display()))?;
            }
        }
    }
    let text = std::fs::read(editor_root.join("fsgame.ltx"))
        .map_err(|e| format!("the editor's fsgame.ltx could not be read: {e}"))?;
    let body = compiler_fsgame(&gz_base::text::cp1251_to_utf8(&text), editor_root, project);
    std::fs::write(stage.join("fsgame.ltx"), gz_base::text::utf8_to_cp1251(&body).0)
        .map_err(|e| format!("the compilers' fsgame.ltx could not be written: {e}"))?;
    Ok(out)
}

/// The compilers' `fsgame.ltx`: the editor's, its roots made absolute, the levels and the spawns
/// the project's; the application data stays beside the stage.
#[must_use]
pub fn compiler_fsgame(text: &str, editor_root: &Path, project: &Path) -> String {
    let root = |p: &Path| format!("{}\\", p.display().to_string().trim_end_matches(['\\', '/']));
    let mut lines = Vec::new();
    for line in text.lines() {
        let alias = line.split('=').next().unwrap_or_default().trim().to_ascii_lowercase();
        lines.push(match alias.as_str() {
            "$game_levels$" => format!("$game_levels$ = true| false| {}| gamedata\\levels\\", root(project)),
            "$game_spawn$" => format!("$game_spawn$ = true| false| {}| gamedata\\spawns\\", root(project)),
            "$app_data_root$" => line.to_owned(),
            _ => line.replace("$fs_root$", &root(editor_root)),
        });
    }
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compilers_read_the_projects_levels() {
        let text = "$game_data$ = false| true| $fs_root$| gamedata\\\n$game_levels$ = true| false| $game_data$| levels\\\n";
        let got = compiler_fsgame(text, Path::new("D:\\Editor"), Path::new("C:\\Temp\\P\\"));
        assert_eq!(
            got,
            "$game_data$ = false| true| D:\\Editor\\| gamedata\\\r\n$game_levels$ = true| false| C:\\Temp\\P\\| gamedata\\levels\\\r\n"
        );
    }
}
