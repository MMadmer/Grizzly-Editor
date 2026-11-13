//! Grizzly Editor.

// A window program: started from Explorer, a shortcut or the taskbar it gets no console, and so
// no console window whose close would end it without the unsaved-work question. A run without
// a window joins its parent's console (`gz_win::attach_parent_console`); one with a window
// never does.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod cli;
mod commandlet;
mod compile;
mod core_cmds;
mod launch;
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

fn main() -> ExitCode {
    let mut startup = Startup::begin();
    let cli = Cli::parse(std::env::args());
    if cli.windowless() {
        gz_win::attach_parent_console();
    }
    let paths = app_paths(&cli);

    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_owned()).to_lowercase();
    // A compile run and a commandlet keep logs of their own: neither takes the log of an editor
    // that is open on the same home.
    let log_name = if cli.compile.is_some() {
        format!("grizzly_compile_{user}.log")
    } else if cli.commandlet() {
        format!("grizzly_cmd_{user}.log")
    } else {
        format!("grizzly_{user}.log")
    };
    let log_file = paths.logs.join(log_name);
    let (log, _log_guard) = match Log::start(Some(log_file.as_path()), cli.echo) {
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
    // one editor per home: a second start hands its project to the editor that holds the home
    // (an update's restart takes over from the one that is ending, and never hands back)
    if !cli.commandlet()
        && cli.updated.is_none()
        && launch::hand_over(&paths.home, cli.project.as_deref(), &log)
    {
        log.info("Grizzly Editor closed: the editor that holds this home took the request");
        return ExitCode::SUCCESS;
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
    let registry = build_registry();

    // a commandlet is the editor without a window
    let headless = cli.headless || cli.commandlet();
    let flags = Flags { unattended: cli.unattended, headless, qa_github: cli.qa_github };
    let editor = Editor::new(EditorOptions { flags, paths }, exec, log.clone(), registry);
    if headless {
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
    // the editor's own updates: a finished one tidied up, a download continued
    let start = gz_update::Start {
        updated_from: cli.updated.clone(),
        window: !headless,
        commandlet: cli.commandlet(),
    };
    gz_update::install(&editor, start);
    gz_scene::install(&editor);
    gz_export_level::install(&editor);
    gz_ui_content::install_hooks(&editor);
    // a project handed over by a second start asks about unsaved work in the window
    gz_ui_app::launch::install_hooks(&editor);
    // the author's own editor keeps .grizzly files opening with it
    let own_home =
        cli.home.is_none() && std::env::var(crate::cli::HOME_VAR).map_or(true, |v| v.trim().is_empty());
    if own_home && !headless && !cli.unattended {
        gz_project::association::at_start(&editor);
    }
    // the Task Center's rows of builds and packaging, also for a build started over MCP
    gz_ui_mod::cards::ensure(&editor);

    // a commandlet listens nowhere unless it is told a port: it is nobody's server
    let mut bound = None;
    if !cli.commandlet() || cli.mcp_port.is_some() {
        startup.stage("Opening the control endpoint");
        let port = cli.mcp_port.unwrap_or(gz_mcp::DEFAULT_PORT);
        // the editor an update restarted into takes the port its predecessor is letting go of
        let mut tries = if cli.updated.is_some() { 50 } else { 1 };
        loop {
            tries -= 1;
            match runtime.block_on(gz_mcp::serve(editor.clone(), port)) {
                Ok(addr) => {
                    log.info(format!("control endpoint: {addr}"));
                    bound = Some(addr.port());
                }
                Err(_) if tries > 0 => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                Err(e) => log.error(format!("the control endpoint did not open on port {port}: {e}")),
            }
            break;
        }
    }

    // the editor holds its home while it runs; a commandlet is nobody's editor
    let _lock = if cli.commandlet() {
        None
    } else {
        bound.and_then(|port| launch::Lock::take(&editor.paths.home, port))
    };

    if cli.commandlet() {
        // it opens its project itself, runs, and ends with what its commands came to
        let code = commandlet::run(&runtime, &editor, &cli);
        runtime.block_on(gz_project::close_project(&editor));
        gz_update::at_exit(&editor);
        log.info("Grizzly Editor closed");
        return code;
    }

    if let Some(project) = &cli.project {
        startup.stage("Opening the project");
        open_project_at_start(&runtime, &editor, project);
    }

    if cli.headless {
        startup.finish(&log);
        gz_update::ready(&editor);
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
            gz_app::ShellOptions {
                fonts,
                iconsheet: cli.iconsheet,
                kitsheet: cli.kitsheet,
                ui_scale: cli.ui_scale,
            },
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
    // a staged update goes in now, and a restart starts the new version
    gz_update::at_exit(&editor);
    log.info("Grizzly Editor closed");
    ExitCode::SUCCESS
}

/// Every command of the editor: each area's own, then the ones the registry itself brings.
fn build_registry() -> Registry {
    let mut registry = Registry::new();
    core_cmds::register(&mut registry);
    // the frame's commands, also headless (they answer "no window" there); an area that owns one
    // of them (ui_panel) registers after this and keeps its own
    gz_app::register(&mut registry);
    gz_project::register(&mut registry);
    gz_game::register(&mut registry);
    gz_mod::register(&mut registry);
    gz_build::register(&mut registry);
    gz_asset::register(&mut registry);
    gz_nq::register(&mut registry);
    gz_release::register(&mut registry);
    gz_publish::register(&mut registry);
    gz_update::register(&mut registry);
    // after gz-release: the real mod_catalog replaces its key-only stand-in
    gz_catalog::register(&mut registry);
    gz_pda::register(&mut registry);
    gz_screens::register(&mut registry);
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
    registry.install_jobs();
    // last: it lists what the areas registered
    registry.install_help();
    registry
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
        tabs_file: gz_project::Project::of(editor).ok().map(|(_, p)| p.tabs_file()),
    }
}

fn open_project_at_start(runtime: &Runtime, editor: &std::sync::Arc<Editor>, project: &Path) {
    let mut args = gz_editor::Args::new();
    args.insert("path".to_owned(), serde_json::Value::String(project.display().to_string()));
    let reply = runtime.block_on(editor.call("project.open", args, editor.ui_origin()));
    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let why = reply.get("error").and_then(serde_json::Value::as_str).unwrap_or("unknown error");
        editor.log.error(format!("the project {} did not open: {why}", project.display()));
    }
}

#[cfg(test)]
mod tests {
    use gz_app::{Drawer, ShellBuilder, contract};

    /// An installation unpacked anywhere has its programs at the data root: `fs.ltx` is beside
    /// them, so the root is their folder and the home defaults to it.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn an_unpacked_installation_is_its_own_data_root() {
        let dir = std::env::temp_dir().join(format!("grizzly-root-{}", std::process::id()));
        let install = dir.join("Grizzly Editor");
        std::fs::create_dir_all(install.join("gamedata")).unwrap();
        std::fs::write(install.join("fs.ltx"), b"$sdk_root$ = false| false| $fs_root$\n").unwrap();
        assert_eq!(super::find_root(&install), install);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command with a card is its own documentation: `help` prints nothing else. So a card has
    /// a name of the agreed form, a summary that is a sentence, and a word on every parameter.
    #[test]
    fn every_card_is_worth_reading() {
        let registry = super::build_registry();
        let mut seen = std::collections::BTreeSet::new();
        let mut wrong = Vec::new();
        for c in registry.commands() {
            assert!(seen.insert(c.name), "{} is registered twice", c.name);
            let Some(spec) = &c.spec else { continue };
            let name = spec.name;
            let well_named = name.split('.').count() <= 2
                && name.split('.').all(|part| {
                    !part.is_empty()
                        && part.starts_with(|ch: char| ch.is_ascii_lowercase())
                        && part.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
                });
            if !well_named {
                wrong.push(format!("{name}: a name is area.verb, lower case"));
            }
            if spec.summary.len() < 12 || !spec.summary.ends_with('.') {
                wrong.push(format!("{name}: the summary is a sentence"));
            }
            if name.contains('.') && !registry.areas().iter().any(|a| a.name == spec.area()) {
                wrong.push(format!("{name}: its area '{}' is not described (Registry::area)", spec.area()));
            }
            let (_, shape) = registry.card(name).unwrap();
            for p in &shape.params {
                if p.doc.is_empty() {
                    wrong.push(format!("{name}: the parameter '{}' has no doc comment", p.name));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{}",
            wrong.join(
                "
"
            )
        );
    }

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
