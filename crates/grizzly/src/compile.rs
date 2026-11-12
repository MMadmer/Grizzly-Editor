//! `-compile <level folder> [flags]` and `-compile_details <level folder>`: the compilers on
//! the command line, without an editor. The log goes to the standard output line by line, each
//! line after the seconds since the start; the exit code is 0 for a level built, 1 otherwise.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use gz_lc::{Input, Log, Options, Sources};
use gz_lc_light::Ctl;

use crate::cli::CompileWhat;

pub fn run(what: CompileWhat, folder: &Path, flags: &[String], root: &Path, log: &gz_log::Log) -> ExitCode {
    let start = Instant::now();
    let echo = move |line: &str| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "[{:9.3}] {line}", start.elapsed().as_secs_f64());
    };
    let (opts, unknown) = match Options::parse(flags.iter().map(String::as_str)) {
        Ok(x) => x,
        Err(e) => {
            echo(&format!("! {e}"));
            log.error(e.to_string());
            return ExitCode::from(1);
        }
    };
    for u in &unknown {
        echo(&format!("! unknown compiler option '{u}'"));
    }
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let pool =
        match rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("lc-{i}")).build()
        {
            Ok(p) => p,
            Err(e) => {
                echo(&format!("! the workers did not start: {e}"));
                return ExitCode::from(1);
            }
        };
    let input = Input { level: folder.to_path_buf(), sources: Sources::of_root(root) };
    echo(&format!("* {} worker threads; data from {}", threads, input.sources.game_data.display()));
    let mut lc_log = Log::with_echo(Box::new(echo));
    let result = pool.install(|| match what {
        CompileWhat::Level => gz_lc::compile(&input, &opts, &Ctl::quiet(), &mut lc_log),
        CompileWhat::Details => gz_lc::compile_details(&input, &Ctl::quiet(), &mut lc_log),
    });
    match result {
        Ok(report) => {
            for (name, size) in &report.files {
                echo(&format!("* written: {name} ({size} bytes)"));
            }
            for (name, s) in &report.phases {
                echo(&format!("* phase {s:9.3} s  {name}"));
            }
            echo(&format!("Build succesful! Time elapsed: {:.3} s", report.seconds));
            log.info(format!("the compile of {} ended in {:.3} s", folder.display(), report.seconds));
            ExitCode::SUCCESS
        }
        Err(e) => {
            echo(&format!("! Build failed: {e}"));
            log.error(format!("the compile of {} failed: {e}", folder.display()));
            ExitCode::from(1)
        }
    }
}
