use std::path::PathBuf;

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
    /// `-home <folder>`: where the editor keeps its project list, settings and logs (the data
    /// root without it). Tests give every editor one of its own.
    pub home: Option<PathBuf>,
    /// `-log_echo`: also print the log to stderr.
    pub echo: bool,
    /// `-iconsheet`: show every icon of the kit and nothing else.
    pub iconsheet: bool,
    /// `-dump_commands`: print every registered command with its area and exit.
    pub dump_commands: bool,
    /// `-uiscale <f>`: the UI at a fixed scale (0.5..4), whatever the monitor says.
    pub ui_scale: Option<f32>,
    /// `-uiscale` was given a value outside 0.5..4 or none (`CommandLineScale`'s message).
    pub bad_ui_scale: bool,
    /// Options nobody knows: named in the log, never fatal.
    pub unknown: Vec<String>,
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
                "uiscale" => match it.next().and_then(|v| v.trim().parse::<f32>().ok()) {
                    Some(v) if (0.5..=4.0).contains(&v) => cli.ui_scale = Some(v),
                    _ => cli.bad_ui_scale = true,
                },
                _ => cli.unknown.push(arg),
            }
        }
        cli
    }
}
