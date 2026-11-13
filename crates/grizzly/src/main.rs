//! Grizzly Editor.

// A window program: started from Explorer, a shortcut or the taskbar it gets no console, and so
// no console window whose close would end it without the unsaved-work question. A run without
// a window joins its parent's console (`gz_win::attach_parent_console`); one with a window
// never does.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod cli;
mod compile;
mod core_cmds;
mod startup;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gz_editor::{AppPaths, Editor, EditorOptions, Flags, Registry};
use gz_log::Log;
use gz_task::{ExecOptions, Runtime};

use crate::cli::Cli;
use crate::startup::Startup;

/// The editor's data root: the nearest folder above the executable that holds the project list
/// or the bundled game data.
fn find_root(exe_dir: &Path) -> PathBuf {
    let mut dir = Some(exe_dir);
    while let Some(d) = dir {
        if d.join("_projects.ini").is_file() || d.join("fs.ltx").is_file() || d.join("gamedata").is_dir() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    std::env::current_dir().unwrap_or_else(|_| exe_dir.to_path_buf())
}

fn app_paths(cli: &Cli) -> AppPaths {
    let exe_dir =
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let root = cli.root.clone().unwrap_or_else(|| find_root(&exe_dir));
    let home = cli.home(std::env::var(crate::cli::HOME_VAR).ok(), &root);
    AppPaths { exe_dir, appdata: root.join("_appdata_"), logs: home.join("logs"), home, root }
}

/// A line for whoever reads this run's output. `false` once the reader is gone (a closed pipe, a
/// stream the shell gave and does not take): the run stops printing and ends as it would have.
fn say(line: &str) -> bool {
    writeln!(std::io::stdout().lock(), "{line}").is_ok()
}

fn main() -> ExitCode {
    let mut startup = Startup::begin();
    let cli = Cli::parse(std::env::args());
    if cli.windowless() {
        gz_win::attach_parent_console();
    }
    // before the log: a check of the binary writes nothing
    if cli.catalogue {
        say(&gz_mcp::catalogue_line());
        if !cli.dump_commands {
            return ExitCode::SUCCESS;
        }
    }
    let paths = app_paths(&cli);

    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_owned()).to_lowercase();
    // A compile run keeps a log of its own, and a dump of the registry writes none: neither takes
    // the log of an editor that is open on the same home.
    let log_name = if cli.compile.is_some() {
        format!("grizzly_compile_{user}.log")
    } else {
        format!("grizzly_{user}.log")
    };
    let log_file = paths.logs.join(log_name);
    let (log, _log_guard) = match Log::start((!cli.dump_commands).then_some(log_file.as_path()), cli.echo) {
        Ok(started) => started,
        Err(e) => {
            gz_win::attach_parent_console();
            let _ = writeln!(
                std::io::stderr().lock(),
                "Grizzly Editor cannot write its log {}: {e}",
                log_file.display()
            );
            return ExitCode::from(2);
        }
    };
    gz_crash::install(
        log.clone(),
        gz_crash::CrashOptions { dir: paths.logs.clone(), unattended: cli.unattended },
    );
    log.info(format!("Grizzly Editor {} starting", env!("CARGO_PKG_VERSION")));
    for arg in &cli.unknown {
        log.warn(format!("unknown command line option '{arg}'"));
    }
    if cli.bad_ui_scale {
        log.warn("-uiscale wants a scale from 0.5 to 4, e.g. -uiscale 1.5");
    }
    if let Some((what, folder, flags)) = &cli.compile {
        return compile::run(*what, folder, flags, &paths.root, &log);
    }

    startup.stage("Starting the workers");
    let runtime = match Runtime::start(ExecOptions::default()) {
        Ok(rt) => rt,
        Err(e) => {
            log.error(format!("the worker threads did not start: {e}"));
            return ExitCode::from(2);
        }
    };
    let exec = runtime.exec();

    startup.stage("Registering commands");
    let mut registry = Registry::new();
    core_cmds::register(&mut registry);
    // the frame's commands, also headless (they answer "no window" there); an area that owns one
    // of them (ui_panel) registers after this and keeps its own
    gz_app::register(&mut registry);
    gz_project::register(&mut registry);
    gz_game::register(&mut registry);
    gz_mod::register(&mut registry);
    gz_build::register(&mut registry);
    gz_nq::register(&mut registry);
    gz_release::register(&mut registry);
    gz_publish::register(&mut registry);
    // after gz-release: the real mod_catalog replaces its key-only stand-in
    gz_catalog::register(&mut registry);
    gz_pda::register(&mut registry);
    gz_screens::register(&mut registry);
    gz_characters::register(&mut registry);
    gz_world::register(&mut registry);
    gz_scene::register(&mut registry);
    gz_export_level::register(&mut registry);
    gz_levelbuild::register(&mut registry);
    gz_ui_app::register(&mut registry);
    gz_ui_mod::register(&mut registry);
    gz_ui_content::register(&mut registry);
    gz_ui_previews::register(&mut registry);
    gz_ui_pda::register(&mut registry);
    // after gz-scene: it wraps property_inspector with the panels' side
    gz_ui_level::register(&mut registry);
    gz_ui_viewport::register(&mut registry);
    gz_ui_graph::register(&mut registry);
    gz_ui_world::register(&mut registry);

    if cli.dump_commands {
        for c in registry.commands() {
            if !say(&format!("{} {}", c.name, c.area)) {
                break;
            }
        }
        return ExitCode::SUCCESS;
    }

    let flags = Flags { unattended: cli.unattended, headless: cli.headless, qa_github: cli.qa_github };
    let editor = Editor::new(EditorOptions { flags, paths }, exec, log.clone(), registry);
    if cli.headless {
        // From here on the end of the console it lives in (Ctrl+C, the console closed) is a quit:
        // whatever is open by then - a project opening at this very start too - closes the way it
        // does for any other.
        let asked = editor.clone();
        gz_win::on_console_end(move || asked.request_quit());
    }
    gz_app::project::set_project_probe(&editor, project_probe);
    gz_game::install(&editor);
    gz_mod::install(&editor);
    gz_nq::install(&editor);
    gz_release::install(&editor);
    gz_pda::install(&editor);
    gz_screens::install(&editor);
    gz_characters::install(&editor);
    gz_world::install(&editor);
    gz_scene::install(&editor);
    gz_export_level::install(&editor);
    gz_ui_content::install_hooks(&editor);
    // the Task Center's rows of builds and packaging, also for a build started over MCP
    gz_ui_mod::cards::ensure(&editor);

    startup.stage("Opening the control endpoint");
    let port = cli.mcp_port.unwrap_or(gz_mcp::DEFAULT_PORT);
    match runtime.block_on(gz_mcp::serve(editor.clone(), port)) {
        Ok(addr) => log.info(format!("control endpoint: {addr}")),
        Err(e) => log.error(format!("the control endpoint did not open on port {port}: {e}")),
    }

    if let Some(project) = &cli.project {
        startup.stage("Opening the project");
        open_project_at_start(&runtime, &editor, project);
    }

    if cli.headless {
        startup.finish(&log);
        // Without a window the editor lives until it is told to quit.
        runtime.block_on(editor.quit_token().cancelled());
    } else {
        startup.stage("Loading fonts");
        let fonts = gz_ui::font::FontFiles::read(&gz_ui::font::system_font_dir());
        startup.stage("Building the shell");
        let mut shell = gz_app::ShellBuilder::new();
        install_ui(&mut shell);
        let mut opts = gz_app::window_options(&editor);
        opts.start_minimized = cli.minimized;
        let shell = gz_app::GrizzlyShell::new(
            editor.clone(),
            shell,
            gz_app::ShellOptions { fonts, iconsheet: cli.iconsheet, ui_scale: cli.ui_scale },
        );
        // the profile runs on to the first frame on the screen
        startup.set_log(log.clone());
        if let Err(e) = gz_app::run(editor.clone(), opts, Box::new(shell), Some(Box::new(startup))) {
            log.error(&e);
            runtime.block_on(gz_project::close_project(&editor));
            return ExitCode::from(2);
        }
    }
    // the project closes with the editor: the scene to reopen, the preview, unsaved graphs
    runtime.block_on(gz_project::close_project(&editor));
    log.info("Grizzly Editor closed");
    ExitCode::SUCCESS
}

/// What the UI crates plug into the shell: each one's `install`, one line per crate.
fn install_ui(shell: &mut gz_app::ShellBuilder) {
    gz_ui_app::install(shell);
    gz_ui_mod::install(shell);
    gz_ui_content::install(shell);
    gz_ui_previews::install(shell);
    gz_ui_release::install(shell);
    gz_ui_pda::install(shell);
    gz_ui_level::install(shell);
    gz_ui_viewport::install(shell);
    gz_ui_graph::install(shell);
    gz_ui_world::install(shell);
}

/// How the shell reads the open project: the project area's own answer.
fn project_probe(editor: &Editor) -> gz_app::project::ProjectInfo {
    let state = gz_project::project_state(editor);
    let text = |k: &str| state.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
    gz_app::project::ProjectInfo {
        active: state.get("project_active").and_then(serde_json::Value::as_bool).unwrap_or(false),
        folder: text("project"),
        name: text("project_name"),
        game_linked: gz_project::Project::of(editor).is_ok_and(|(_, p)| p.game_linked_now()),
    }
}

fn open_project_at_start(runtime: &Runtime, editor: &std::sync::Arc<Editor>, project: &Path) {
    let mut args = gz_editor::Args::new();
    args.insert("path".to_owned(), serde_json::Value::String(project.display().to_string()));
    let reply = runtime.block_on(editor.call("open_project", args, editor.ui_origin()));
    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let why = reply.get("error").and_then(serde_json::Value::as_str).unwrap_or("unknown error");
        editor.log.error(format!("the project {} did not open: {why}", project.display()));
    }
}

#[cfg(test)]
mod tests {
    use gz_app::{Drawer, ShellBuilder, contract};

    /// The shell keeps a sentence for a surface nobody installed (a test of `gz-app` alone):
    /// the editor installs every one of them, so none of those sentences is ever its answer.
    #[test]
    fn the_editor_installs_every_surface_the_contract_names() {
        let mut shell = ShellBuilder::new();
        super::install_ui(&mut shell);
        for kind in contract::DOCUMENT_KINDS {
            assert!(shell.has_document(kind), "no tab of the kind {kind}");
        }
        for (id, _) in contract::DIALOGS {
            assert!(shell.has_dialog(id), "no dialog {id}");
        }
        assert!(shell.has_drawer(Drawer::Content) && shell.has_drawer(Drawer::Log));
        assert!(shell.has_start_page());
        // the scene's file rows are the Level tab's, with its Open Scene and Save As dialogs
        assert!(shell.cpp_subs("COMMAND_LOAD").is_some());
        assert_eq!(shell.cpp_subs("COMMAND_SAVE"), Some(vec!["Save".to_owned(), "Save As".to_owned()]));
    }
}
