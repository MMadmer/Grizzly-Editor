//! The level compiler on a level folder without the editor: what every phase costs and how fast
//! a lighter is. A measuring tool; it writes nothing into the level folder.
//!
//! ```text
//! lc_bench <level folder> root=<data root> [light=cpu|gpu|auto|off] [mode=compile|rays|details]
//!          [stop_after=<phase>] [threads=N] [samples=N] [compare=1] [out=<folder>] [<compiler flags>]
//! ```
//!
//! - `<level folder>` holds `build.prj` (and `build.details` for `mode=details`);
//! - `root=` is the editor's data root (`gamedata\shaders_xrlc.xr`, the textures) - the main
//!   checkout, not a worktree;
//! - `mode=compile` (the default) runs the whole compile in memory and prints every phase's
//!   seconds and the SHA-256 and size of every file it made; `stop_after=` ends it when the phase
//!   whose name starts so has ended; `out=` also writes the files into that folder;
//! - `mode=rays` builds the scene the light is traced in, lights `samples=` of its vertices
//!   (200000) with the whole light set and prints the samples a second; `compare=1` lights them
//!   on the CPU too and says how far the two are apart;
//! - `mode=details` lights the detail slots from `build.details`, `build.cform` and
//!   `build.lights` (the last two from `out=` when it holds them, else from the level folder);
//! - a word without `=` is a compiler flag (`nosun`, `no_invalidefaces`, ...).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use gz_fmt_level::BuildProject;
use gz_lc::{Log, Options, PHASE_DONE, Sources};
use gz_lc_light::{ColorC, CpuLighter, Ctl, Falloff, LP_DEFAULT, LightJob, Lighter, Sample, TextureUse};
use gz_task::Cancel;
use sha2::{Digest, Sha256};

/// A line to stdout; a reader that left is no error.
macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

struct Args {
    folder: PathBuf,
    root: PathBuf,
    mode: String,
    stop_after: Option<String>,
    samples: usize,
    compare: bool,
    out: Option<PathBuf>,
    threads: Option<usize>,
    flags: Vec<String>,
}

fn args() -> Result<Args, String> {
    let mut a = Args {
        folder: PathBuf::new(),
        root: PathBuf::new(),
        mode: "compile".to_owned(),
        stop_after: None,
        samples: 200_000,
        compare: false,
        out: None,
        threads: None,
        flags: Vec::new(),
    };
    for word in std::env::args().skip(1) {
        match word.split_once('=') {
            Some(("root", v)) => a.root = PathBuf::from(v),
            Some(("mode", v)) => a.mode = v.to_owned(),
            Some(("stop_after", v)) => a.stop_after = Some(v.to_ascii_lowercase()),
            Some(("samples", v)) => a.samples = v.parse().map_err(|_| format!("{word}: a whole number"))?,
            Some(("compare", v)) => a.compare = v != "0",
            Some(("out", v)) => a.out = Some(PathBuf::from(v)),
            Some(("threads", v)) => {
                a.threads = Some(v.parse().map_err(|_| format!("{word}: a whole number"))?);
            }
            // light= and the compiler's own switches
            Some(_) => a.flags.push(word),
            None if a.folder.as_os_str().is_empty() && !word.starts_with('-') => {
                a.folder = PathBuf::from(word)
            }
            None => a.flags.push(word),
        }
    }
    if a.folder.as_os_str().is_empty() || a.root.as_os_str().is_empty() {
        return Err("usage: lc_bench <level folder> root=<data root> [light=cpu|gpu|auto|off] \
                    [mode=compile|rays|details] [stop_after=<phase>] [threads=N] [samples=N] [compare=1] \
                    [out=<folder>] [<compiler flags>]"
            .to_owned());
    }
    Ok(a)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{} could not be read: {e}", path.display()))
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn phases(log: &Log) {
    for (name, seconds) in &log.phases {
        say!("phase {seconds:9.3} s  {name}");
    }
}

fn said_by(lighter: &dyn Lighter) {
    for note in lighter.notes() {
        say!("note   {note}");
    }
    say!("lighter {}: {} samples lit", lighter.name(), lighter.samples());
}

/// The whole compile in memory.
fn compile(a: &Args, prj: &BuildProject, opts: &Options, lighter: &dyn Lighter) -> Result<(), String> {
    let sources = Sources::of_root(&a.root);
    let shaders = sources.shaders().map_err(|e| e.to_string())?;
    let textures =
        sources.textures(&prj.textures, TextureUse::Level { radiosity: false }).map_err(|e| e.to_string())?;
    let cancel = Cancel::new();
    let (stop, stopper) = (a.stop_after.clone(), cancel.clone());
    let echo = move |line: &str| {
        if let (Some(stop), Some(done)) = (&stop, line.strip_prefix(PHASE_DONE))
            && done.to_ascii_lowercase().starts_with(stop.as_str())
        {
            stopper.cancel();
        }
    };
    let mut log = Log::with_echo(Box::new(echo));
    let ctl = Ctl::with_cancel(&cancel);
    let t = Instant::now();
    let made = gz_lc::compile_project(prj, shaders, textures, opts, lighter, &ctl, &mut log);
    let seconds = t.elapsed().as_secs_f64();
    phases(&log);
    said_by(lighter);
    match made {
        Ok(out) => {
            for (name, bytes) in &out.files {
                say!("file  {} {:>10}  {name}", sha(bytes), bytes.len());
            }
            if let Some(dir) = &a.out {
                for (name, bytes) in &out.files {
                    let path = dir.join(name.replace('\\', "/"));
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                }
            }
            say!("total {seconds:9.3} s  ({} invalid faces)", out.invalid_faces);
            Ok(())
        }
        Err(e) if e.cancelled() => {
            say!("stopped after {seconds:.3} s, as asked");
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The detail slots' light in memory.
fn details(a: &Args, prj: &BuildProject, lighter: &dyn Lighter) -> Result<(), String> {
    let sources = Sources::of_root(&a.root);
    let shaders = sources.shaders().map_err(|e| e.to_string())?;
    let textures = sources.textures(&prj.textures, TextureUse::Details).map_err(|e| e.to_string())?;
    let beside = |name: &str| match &a.out {
        Some(dir) if dir.join(name).is_file() => dir.join(name),
        _ => a.folder.join(name),
    };
    let slots = read(&a.folder.join("build.details"))?;
    let (cform, lights) = (read(&beside("build.cform"))?, read(&beside("build.lights"))?);
    let mut log = Log::default();
    let t = Instant::now();
    let bytes = gz_lc::compile_details_in_memory(
        prj,
        shaders,
        textures,
        &slots,
        &cform,
        &lights,
        lighter,
        &Ctl::quiet(),
        &mut log,
    )
    .map_err(|e| e.to_string())?;
    said_by(lighter);
    say!("file  {} {:>10}  level.details", sha(&bytes), bytes.len());
    say!("total {:9.3} s", t.elapsed().as_secs_f64());
    Ok(())
}

/// How fast a lighter lights the scene's own vertices with the whole light set.
fn rays(a: &Args, prj: &BuildProject, opts: &Options, lighter: &dyn Lighter) -> Result<(), String> {
    let sources = Sources::of_root(&a.root);
    let shaders = sources.shaders().map_err(|e| e.to_string())?;
    let textures =
        sources.textures(&prj.textures, TextureUse::Level { radiosity: false }).map_err(|e| e.to_string())?;
    let t = Instant::now();
    let g = gz_lc::light_scene(prj, shaders, textures, opts, &Ctl::quiet()).map_err(|e| e.to_string())?;
    let model = g.ray.as_ref().ok_or("the ray model was not built")?;
    say!(
        "scene {:9.3} s  the structure up to the ray model: {} triangles",
        t.elapsed().as_secs_f64(),
        model.len()
    );
    let verts = g.mesh.vertices();
    let stride = (verts.len() / a.samples.max(1)).max(1);
    let samples: Vec<Sample> = verts
        .iter()
        .step_by(stride)
        .filter_map(|&v| g.mesh.vertex(v))
        .filter(|x| x.n.length_squared() > 0.25)
        .map(|x| Sample::new(x.p, x.n.normalize(), LP_DEFAULT, None))
        .collect();
    let job = LightJob { lights: &g.lights, falloff: Falloff::Level { linear: g.linear } };
    let per_sample = g.lights.rgb.len() + g.lights.hemi.len() + g.lights.sun.len();
    say!(
        "lights: {} static, {} of the hemisphere, {} of the sun ({per_sample} rays a sample at most)",
        g.lights.rgb.len(),
        g.lights.hemi.len(),
        g.lights.sun.len()
    );
    let t = Instant::now();
    let scene = lighter.scene(model, &g.textures).map_err(|e| e.to_string())?;
    say!("ready {:9.3} s  the lighter's scene", t.elapsed().as_secs_f64());
    let mut out = vec![ColorC::default(); samples.len()];
    let t = Instant::now();
    scene.light(&job, &samples, &mut out, &Ctl::quiet()).map_err(|e| e.to_string())?;
    let seconds = t.elapsed().as_secs_f64().max(1e-9);
    drop(scene);
    said_by(lighter);
    let sum = out.iter().fold([0.0f64; 3], |s, c| {
        [s[0] + f64::from(c.hemi), s[1] + f64::from(c.sun), s[2] + f64::from(c.rgb.x + c.rgb.y + c.rgb.z)]
    });
    say!(
        "light {seconds:9.3} s  {} samples, {:.0} samples a second (at most {:.1} million rays a second)",
        samples.len(),
        samples.len() as f64 / seconds,
        samples.len() as f64 * per_sample as f64 / seconds / 1e6
    );
    say!("sums: hemisphere {:.3}, sun {:.3}, static {:.3}", sum[0], sum[1], sum[2]);
    if a.compare {
        let cpu = CpuLighter::default();
        let mut want = vec![ColorC::default(); samples.len()];
        let t = Instant::now();
        cpu.scene(model, &g.textures)
            .and_then(|s| s.light(&job, &samples, &mut want, &Ctl::quiet()))
            .map_err(|e| e.to_string())?;
        let cpu_seconds = t.elapsed().as_secs_f64().max(1e-9);
        let apart = |a: &ColorC, b: &ColorC| {
            (a.rgb - b.rgb).abs().max_element().max((a.hemi - b.hemi).abs()).max((a.sun - b.sun).abs())
        };
        let within = out.iter().zip(&want).filter(|(a, b)| apart(a, b) <= 0.002).count();
        let mean = out.iter().zip(&want).map(|(a, b)| f64::from(apart(a, b))).sum::<f64>()
            / samples.len().max(1) as f64;
        say!(
            "cpu   {cpu_seconds:9.3} s  {:.0} samples a second: the lighter is {:.1} times that",
            samples.len() as f64 / cpu_seconds,
            cpu_seconds / seconds
        );
        say!(
            "apart: {:.3} % of the samples within 0.002 of the CPU's, the mean difference {mean:.6}",
            100.0 * within as f64 / samples.len().max(1) as f64
        );
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let a = args()?;
    let flags: Vec<&str> = a.flags.iter().map(String::as_str).collect();
    let (opts, unknown) = Options::parse(flags).map_err(|e| e.to_string())?;
    if !unknown.is_empty() {
        return Err(format!("the compiler does not know: {}", unknown.join(" ")));
    }
    let prj = BuildProject::read(&read(&a.folder.join("build.prj"))?).map_err(|e| e.to_string())?;
    let chosen = gz_lc_gpu::lighter(opts.light)?;
    if let Some(note) = &chosen.note {
        say!("note   {note}");
    }
    let lighter = chosen.lighter.as_ref();
    let work = || match a.mode.as_str() {
        "compile" => compile(&a, &prj, &opts, lighter),
        "rays" => rays(&a, &prj, &opts, lighter),
        "details" => details(&a, &prj, lighter),
        other => Err(format!("mode={other}: compile, rays or details")),
    };
    match a.threads {
        Some(n) => rayon_pool(n)?.install(work),
        None => work(),
    }
}

fn rayon_pool(threads: usize) -> Result<rayon::ThreadPool, String> {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr().lock(), "lc_bench: {e}");
            ExitCode::FAILURE
        }
    }
}
