use std::path::{Path, PathBuf};

/// The environment variable that names the home when `-home` does not.
pub const HOME_VAR: &str = "GRIZZLY_HOME";

/// The command line. Options keep the single dash the editor has always taken (`-project`,
/// `-nodlg`); a double dash is accepted too.
#[derive(Clone, Debug, Default)]
pub struct Cli {
    /// `-project <folder>`: open this project at start.
    pub project: Option<PathBuf>,
    /// `-nodlg`: no dialog ever opens; a crash is a stack, a dump and exit code 3.
    pub unattended: bool,
    /// `-headless`: no window; everything but the picture, driven over MCP.
    pub headless: bool,
    /// `-qa_github`: network clients talk to the mock the environment names.
    pub qa_github: bool,
    /// `-mcp_port <n>`: where the control endpoint listens. 0 lets the system pick.
    pub mcp_port: Option<u16>,
    /// `-root <folder>`: the editor's data root, when it is not found from the executable.
    pub root: Option<PathBuf>,
    /// `-home <folder>`: where the editor keeps its project list, settings and logs. Without it
    /// `GRIZZLY_HOME`, then the data root. Tests give every editor one of its own.
    pub home: Option<PathBuf>,
    /// `-log_echo`: also print the log to stderr.
    pub echo: bool,
    /// `-iconsheet`: show every icon of the kit and nothing else.
    pub iconsheet: bool,
    /// `-dump_commands`: print every registered command with its area and exit.
    pub dump_commands: bool,
    /// `-catalogue`: print the line of the MCP catalogue this build embeds
    /// (`gz_mcp::catalogue_line`) and exit; with `-dump_commands` the commands follow it.
    pub catalogue: bool,
    /// `-uiscale <f>`: the UI at a fixed scale (0.5..4), whatever the monitor says.
    pub ui_scale: Option<f32>,
    /// `-uiscale` was given a value outside 0.5..4 or none (`CommandLineScale`'s message).
    pub bad_ui_scale: bool,
    /// Options nobody knows: named in the log, never fatal.
    pub unknown: Vec<String>,
    /// `-compile <level folder> [flags]`: the level compiler on the folder's `build.prj`, then
    /// exit (`-compile_details`: the detail compiler). Every option after the folder is the
    /// compiler's.
    pub compile: Option<(CompileWhat, PathBuf, Vec<String>)>,
}

/// Which compiler `-compile` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileWhat {
    Level,
    Details,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut cli = Cli::default();
        let mut it = args.into_iter().skip(1);
        while let Some(arg) = it.next() {
            let name = arg.trim_start_matches('-').to_ascii_lowercase();
            if !arg.starts_with('-') {
                cli.unknown.push(arg);
                continue;
            }
            match name.as_str() {
                "project" => cli.project = it.next().map(PathBuf::from),
                "nodlg" => cli.unattended = true,
                "headless" => cli.headless = true,
                "qa_github" => cli.qa_github = true,
                "mcp_port" | "mcp-port" => cli.mcp_port = it.next().and_then(|v| v.parse().ok()),
                "root" => cli.root = it.next().map(PathBuf::from),
                "home" => cli.home = it.next().map(PathBuf::from),
                "log_echo" | "log-echo" => cli.echo = true,
                "iconsheet" => cli.iconsheet = true,
                "dump_commands" => cli.dump_commands = true,
                "catalogue" => cli.catalogue = true,
                "compile" | "compile_details" => {
                    let what = if name == "compile" { CompileWhat::Level } else { CompileWhat::Details };
                    if let Some(folder) = it.next() {
                        let flags: Vec<String> = it.by_ref().collect();
                        cli.compile = Some((what, PathBuf::from(folder), flags));
                    } else {
                        cli.unknown.push(arg);
                    }
                }
                "uiscale" => match it.next().and_then(|v| v.trim().parse::<f32>().ok()) {
                    Some(v) if (0.5..=4.0).contains(&v) => cli.ui_scale = Some(v),
                    _ => cli.bad_ui_scale = true,
                },
                _ => cli.unknown.push(arg),
            }
        }
        cli
    }

    /// Whether this run talks to whoever started it: a check of the binary, the compilers, the
    /// log echo, or a headless editor that lives on the command line. Such a run takes its
    /// parent's console for the streams the launcher did not redirect. A window run keeps out of
    /// any console, so closing the terminal it was started from cannot end it.
    pub fn speaks(&self) -> bool {
        self.catalogue || self.dump_commands || self.compile.is_some() || self.echo || self.headless
    }

    /// The home: `-home`, else `env_home` (the value of [`HOME_VAR`]) when it is not blank, else the
    /// data root. A test harness sets the variable once and every editor it starts keeps out of
    /// the repository's own project list and settings, whether its command line names a home or not.
    pub fn home(&self, env_home: Option<String>, root: &Path) -> PathBuf {
        self.home
            .clone()
            .or_else(|| env_home.filter(|v| !v.trim().is_empty()).map(|v| PathBuf::from(v.trim())))
            .unwrap_or_else(|| root.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse(std::iter::once("grizzly").chain(args.iter().copied()).map(str::to_owned))
    }

    #[test]
    fn the_home_comes_from_the_command_line_then_the_environment_then_the_root() {
        let root = Path::new("C:/editor");
        assert_eq!(cli(&["-home", "C:/a"]).home(Some("C:/b".into()), root), PathBuf::from("C:/a"));
        assert_eq!(cli(&[]).home(Some("C:/b".into()), root), PathBuf::from("C:/b"));
        assert_eq!(cli(&[]).home(Some("  ".into()), root), root);
        assert_eq!(cli(&[]).home(None, root), root);
    }

    #[test]
    fn the_compiler_takes_every_option_after_its_folder() {
        let c = cli(&["-headless", "-compile", "D:/levels/x", "-nosun", "-noise"]);
        let (what, folder, flags) = c.compile.unwrap();
        assert_eq!(what, CompileWhat::Level);
        assert_eq!(folder, PathBuf::from("D:/levels/x"));
        assert_eq!(flags, vec!["-nosun".to_owned(), "-noise".to_owned()]);
        assert!(c.headless);
    }

    #[test]
    fn only_a_run_that_prints_takes_a_console() {
        for args in
            [&["-dump_commands"][..], &["-catalogue"], &["-log_echo"], &["-headless"], &["-compile", "D:/x"]]
        {
            assert!(cli(args).speaks(), "{args:?}");
        }
        assert!(!cli(&[]).speaks());
        assert!(!cli(&["-project", "D:/p", "-nodlg", "-mcp_port", "28917"]).speaks());
    }
}
