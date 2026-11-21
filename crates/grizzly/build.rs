//! The executable's link settings, its icon and the commit it is built from.

use std::path::Path;
use std::process::Command;

/// `GRIZZLY_BUILD_COMMIT`: the commit a report names (gz-feedback). git says where its own files are
/// (a worktree's `.git` is a file); only the files that exist are watched, or the script would run at
/// every build.
fn commit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name])
            && Path::new(&path).is_file()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &reference])
        && Path::new(&path).is_file()
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=GRIZZLY_BUILD_COMMIT={commit}");
}

fn main() {
    // the files this script reads: told to cargo, which would otherwise run it after any change
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/grizzly-editor-icon.ico");
    commit();
    // One C runtime: the DLL one (ucrt, vcruntime140) the standard library and the Vorbis encoder
    // are built for. LuaJIT comes as a static library compiled for the static runtime
    // (luajit-src runs `msvcbuild.bat static`); that runtime is left out, so its calls go to the
    // same DLL runtime. With both, the thin-LTO release link takes the static runtime first and
    // leaves the DLL runtime's imports (`__imp_realloc`) undefined.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        for lib in ["libcmt.lib", "libucrt.lib", "libvcruntime.lib"] {
            println!("cargo:rustc-link-arg-bins=/NODEFAULTLIB:{lib}");
        }
    }
    // the editor's one icon, the picture Explorer and the taskbar show for Grizzly.exe
    let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/grizzly-editor-icon.ico");
    if let Err(e) = gz_ico::embed(&ico) {
        println!("cargo:warning=the icon: {e}");
        std::process::exit(1);
    }
}
