//! The executable's link settings and its icon.

fn main() {
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
