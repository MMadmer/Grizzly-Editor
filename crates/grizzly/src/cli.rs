use std::path::{Path, PathBuf};

/// The environment variable that names the home when `-home` does not.
pub const HOME_VAR: &str = "GRIZZLY_HOME";

/// What a run without a window is asked to do: the first word of the command line that is not
/// an option. A commandlet opens the project, runs, prints and ends (`commandlet.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `run <command> [name=value ...]`: one command.
    Run { command: String, args: Vec<(String, String)> },
    /// `script <file>`: a command per line; `-` reads the standard input.
    Script { file: PathBuf },
    /// `help [area | command | topic]`.
    Help { about: Option<String> },
    /// `serve`: the editor without a window, driven over its control endpoint (as `-headless`).
    Serve,
}

/// A short name of a command a shell runs often: `check [path]` is `run asset.check path=...`.
struct Alias {
    word: &'static str,
    command: &'static str,
    /// The parameter the alias's one bare argument is the value of.
    bare: Option<&'static str>,
    /// Parameters the alias always sends unless the command line names them.
    fixed: &'static [(&'static str, &'static str)],
}

const ALIASES: [Alias; 5] = [
    Alias { word: "check", command: "asset.check", bare: Some("path"), fixed: &[("fail_on", "error")] },
    Alias { word: "reference", command: "asset.reference", bare: Some("kind"), fixed: &[] },
    Alias { word: "build", command: "mod.build", bare: Some("target"), fixed: &[] },
    Alias { word: "package", command: "mod.package", bare: None, fixed: &[] },
    Alias { word: "update", command: "update.apply", bare: None, fixed: &[("restart", "false")] },
];

/// The command line. Options keep the single dash the editor has always taken (`-project`,
/// `-nodlg`); a double dash is accepted too.
#[derive(Clone, Debug, Default)]
pub struct Cli {
    /// `-project <folder or project file>`, or a bare path: open this project at start.
    pub project: Option<PathBuf>,
    /// `-nodlg`: no dialog ever opens; a crash is a stack, a dump and exit code 3.
    pub unattended: bool,
    /// `-headless`: no window; everything but the picture, driven over the control endpoint.
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
    /// `-kitsheet`: show every widget of the kit in every state, and its tokens, and nothing else.
    pub kitsheet: bool,
    /// `-minimized`: the window starts minimised, as it does for a launcher that asks for a
    /// minimised start (a shortcut's Run: Minimized). The twin for a command line passes its own
    /// launcher's request on with it.
    pub minimized: bool,
    /// `-uiscale <f>`: the UI at a fixed scale (0.5..4), whatever the monitor says.
    pub ui_scale: Option<f32>,
    /// `-uiscale` was given a value outside 0.5..4 or none.
    pub bad_ui_scale: bool,
    /// Options nobody knows: named in the log, never fatal.
    pub unknown: Vec<String>,
    /// The commandlet this run is, when it is one.
    pub verb: Option<Verb>,
    /// `-json`: a commandlet prints every answer as one line of JSON.
    pub json: bool,
    /// `-quiet`: a commandlet does not print the progress of jobs.
    pub quiet: bool,
    /// `-keep_going`: a script runs past a command that failed.
    pub keep_going: bool,
    /// Why the commandlet's own words make no sense; the run prints it and ends with 2.
    pub usage: Option<String>,
    /// `-updated <version>`: this run is the new version an update restarted into, from that one.
    pub updated: Option<String>,
}

impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut cli = Cli::default();
        // the words that are not options, in order: a verb and what belongs to it, or a project
        let mut words: Vec<String> = Vec::new();
        let mut it = args.into_iter().skip(1);
        while let Some(arg) = it.next() {
            // `-` alone is a script's standard input, and a negative number is a value
            if !arg.starts_with('-') || arg == "-" || arg.contains('=') {
                words.push(arg);
                continue;
            }
            let name = arg.trim_start_matches('-').to_ascii_lowercase();
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
                "kitsheet" => cli.kitsheet = true,
                "minimized" => cli.minimized = true,
                "json" => cli.json = true,
                "quiet" => cli.quiet = true,
                "keep_going" | "keep-going" => cli.keep_going = true,
                "updated" => cli.updated = it.next(),
                "uiscale" => match it.next().and_then(|v| v.trim().parse::<f32>().ok()) {
                    Some(v) if (0.5..=4.0).contains(&v) => cli.ui_scale = Some(v),
                    _ => cli.bad_ui_scale = true,
                },
                _ => cli.unknown.push(arg),
            }
        }
        cli.take_words(words);
        cli
    }

    /// The words that are not options: a commandlet's verb with its own words, else a project
    /// to open (what Explorer passes for a double-clicked project file).
    fn take_words(&mut self, words: Vec<String>) {
        let mut words = words.into_iter();
        let Some(first) = words.next() else { return };
        let rest: Vec<String> = words.collect();
        match first.to_ascii_lowercase().as_str() {
            "run" => {
                let mut rest = rest.into_iter();
                match rest.next() {
                    Some(command) if !command.contains('=') => self.run(command, &[], rest.collect()),
                    _ => self.usage = Some("run <command> [name=value ...]".to_owned()),
                }
            }
            "script" => match rest.as_slice() {
                [file] => self.verb = Some(Verb::Script { file: PathBuf::from(file) }),
                _ => self.usage = Some("script <file>   (- reads the standard input)".to_owned()),
            },
            "help" => match rest.as_slice() {
                [] => self.verb = Some(Verb::Help { about: None }),
                [about] => self.verb = Some(Verb::Help { about: Some(about.clone()) }),
                _ => self.usage = Some("help [area | command | topic]".to_owned()),
            },
            "serve" if rest.is_empty() => {
                self.verb = Some(Verb::Serve);
                self.headless = true;
            }
            word => {
                if let Some(alias) = ALIASES.iter().find(|a| a.word == word) {
                    let mut given = rest;
                    // the alias's one bare word is the value of its parameter
                    if let (Some(param), Some(i)) = (alias.bare, given.iter().position(|w| !w.contains('=')))
                    {
                        let value = given.remove(i);
                        given.push(format!("{param}={value}"));
                    }
                    self.run(alias.command.to_owned(), alias.fixed, given);
                } else if rest.is_empty() && self.project.is_none() {
                    self.project = Some(PathBuf::from(first));
                } else {
                    self.unknown.push(first);
                    self.unknown.extend(rest);
                }
            }
        }
    }

    fn run(&mut self, command: String, fixed: &[(&str, &str)], words: Vec<String>) {
        let mut args: Vec<(String, String)> =
            fixed.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        for word in words {
            match word.split_once('=') {
                Some((name, value)) if !name.is_empty() => {
                    args.retain(|(k, _)| k != name);
                    args.push((name.to_owned(), value.to_owned()));
                }
                _ => {
                    self.usage = Some(format!("'{word}': write a parameter as name=value"));
                    return;
                }
            }
        }
        self.verb = Some(Verb::Run { command, args });
    }

    /// Whether this run is a commandlet: it runs commands and ends, without a window and
    /// without a control endpoint of its own.
    pub fn commandlet(&self) -> bool {
        self.usage.is_some()
            || matches!(self.verb, Some(Verb::Run { .. } | Verb::Script { .. } | Verb::Help { .. }))
    }

    /// Whether this run has no window: a commandlet or a headless editor. Such a run
    /// lives in its launcher's console the way a console program does - it prints there what the
    /// launcher did not redirect, and the console's end is its end. A run with a window keeps
    /// out of every console, `-log_echo` or not (its echo goes to the streams it was given): no
    /// console can end it past the unsaved-work question.
    pub fn windowless(&self) -> bool {
        self.commandlet() || self.headless
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

    fn run(command: &str, args: &[(&str, &str)]) -> Option<Verb> {
        Some(Verb::Run {
            command: command.to_owned(),
            args: args.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        })
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
    fn only_a_run_without_a_window_lives_in_a_console() {
        for args in [&["help"][..], &["run", "ping"], &["-headless"], &["serve"]] {
            assert!(cli(args).windowless(), "{args:?}");
        }
        assert!(cli(&["-headless", "-log_echo"]).windowless());
        assert!(!cli(&[]).windowless());
        // the echo of a window run goes to the streams it was given, never to a console it joins
        assert!(!cli(&["-log_echo"]).windowless());
        assert!(!cli(&["-project", "D:/p", "-nodlg", "-mcp_port", "28917"]).windowless());
        // a project file handed over by Explorer is a run with a window
        assert!(!cli(&["D:/p/Mine.grizzly"]).windowless());
    }

    #[test]
    fn the_first_word_is_a_commandlet_or_a_project() {
        let c = cli(&["run", "scene.place", "kind=object", "position=1,2,-3", "-project", "D:/p", "-json"]);
        assert_eq!(c.verb, run("scene.place", &[("kind", "object"), ("position", "1,2,-3")]));
        assert_eq!(c.project, Some(PathBuf::from("D:/p")));
        assert!(c.json && c.commandlet() && c.usage.is_none());
        assert_eq!(cli(&["help"]).verb, Some(Verb::Help { about: None }));
        assert_eq!(cli(&["help", "scene"]).verb, Some(Verb::Help { about: Some("scene".into()) }));
        assert_eq!(cli(&["script", "-"]).verb, Some(Verb::Script { file: PathBuf::from("-") }));
        let serve = cli(&["serve", "-mcp_port", "0"]);
        assert!(serve.headless && !serve.commandlet() && serve.verb == Some(Verb::Serve));
        // a bare path is the project to open, as a double click passes it
        let open = cli(&["D:/Projects/Mine/Mine.grizzly"]);
        assert_eq!(open.project, Some(PathBuf::from("D:/Projects/Mine/Mine.grizzly")));
        assert!(open.verb.is_none() && !open.commandlet());
    }

    #[test]
    fn a_short_word_is_a_command_with_its_parameters() {
        assert_eq!(cli(&["check"]).verb, run("asset.check", &[("fail_on", "error")]));
        assert_eq!(
            cli(&["check", "quests/a.nqasset"]).verb,
            run("asset.check", &[("fail_on", "error"), ("path", "quests/a.nqasset")])
        );
        // a parameter given by name replaces the alias's own
        assert_eq!(cli(&["check", "fail_on=warning"]).verb, run("asset.check", &[("fail_on", "warning")]));
        assert_eq!(cli(&["reference", "quest"]).verb, run("asset.reference", &[("kind", "quest")]));
        assert_eq!(cli(&["update"]).verb, run("update.apply", &[("restart", "false")]));
        // the restart into a new version names the old one; it is no project to open
        let updated = cli(&["-updated", "0.7.0", "-headless"]);
        assert_eq!(updated.updated.as_deref(), Some("0.7.0"));
        assert!(updated.project.is_none() && updated.unknown.is_empty());
    }

    #[test]
    fn words_that_make_no_command_are_a_usage_line() {
        assert_eq!(cli(&["run"]).usage.as_deref(), Some("run <command> [name=value ...]"));
        assert_eq!(cli(&["run", "a", "b"]).usage.as_deref(), Some("'b': write a parameter as name=value"));
        assert!(cli(&["script"]).usage.is_some());
        assert!(cli(&["run"]).commandlet());
    }
}
